//! Template

#![no_std]
#![no_main]

use core::{cell::RefCell, panic};

use defmt::*;
use embassy_embedded_hal::shared_bus::blocking::spi::SpiDeviceWithConfig;
use embassy_executor::Spawner;
use embassy_rp::{
    gpio::{Level, Output},
    i2c::{self, I2c},
    peripherals::USB,
    spi::{self, Spi},
};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_time::{Instant, Timer};
//use embedded_hal_1::i2c::I2c;

use usb_pd::{
    header::{DataMessageType, Header, SpecificationRevision},
    pdo::{
        self, CertStatVDO, PowerDataObject, ProductVDO, UFPTypeVDO, UFPVDOVersion, USBHighestSpeed,
        VDMHeader, VDMVersionMajor, VDMVersionMinor, VconnPower, VendorDataObject,
    },
    sink::{Event, Request, Sink},
};

use {defmt_rtt as _, panic_probe as _};

const FLASH_SIZE: usize = 16 * 1024 * 1024;
extern "C" {
    // Flash storage used for configuration
    static __storage_a_start: u32;
    static __storage_b_start: u32;
}

type Fusb = fusb302b::Fusb302b<i2c::I2c<'static, embassy_rp::peripherals::I2C0, i2c::Blocking>>;
type PDSink = Sink<Fusb>;

struct PDOList {
    pdos: heapless::Vec<PowerDataObject, 8>,
    accepted: Option<usize>,
    ready: Option<usize>,
    requested: Option<usize>,
    rejected: Option<usize>,
}

static PDO_LIST: Mutex<CriticalSectionRawMutex, RefCell<PDOList>> =
    Mutex::new(RefCell::new(PDOList {
        pdos: heapless::Vec::new(),
        accepted: None,
        ready: None,
        requested: None,
        rejected: None,
    }));

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    info!("Hello World!");
    info!("Flash size: {} bytes", FLASH_SIZE);
    info!("Storage A: 0x{:08x}", unsafe {
        &__storage_a_start as *const _ as u32
    });
    info!("Storage B: 0x{:08x}", unsafe {
        &__storage_b_start as *const _ as u32
    });

    // Initialize I2C0 and FUSB302B
    let i2c0_sda = p.PIN_0;
    let i2c0_scl = p.PIN_1;

    let i2c0 = I2c::new_blocking(p.I2C0, i2c0_scl, i2c0_sda, i2c::Config::default());
    let fusb = fusb302b::Fusb302b::new(i2c0);
    let mut sink = Sink::new(fusb);

    sink.init();

    // Spawn PD task
    spawner.spawn(pd_task(sink)).unwrap();

    // Initialize SPI0 and display
    let spi0_tx = p.PIN_7;
    let spi0_rx = p.PIN_4;
    let spi0_sck = p.PIN_6;

    let dis_cs = p.PIN_5;
    let dis_dc = p.PIN_9;
    let dis_rst = p.PIN_10;

    let mut spi0_config = spi::Config::default();
    spi0_config.frequency = 64_000_000; // 64 MHz
    spi0_config.phase = spi::Phase::CaptureOnSecondTransition;
    spi0_config.polarity = spi::Polarity::IdleHigh;

    let spi0 = Spi::new_blocking(p.SPI0, spi0_sck, spi0_tx, spi0_rx, spi0_config.clone());
    let spi0_bus: Mutex<CriticalSectionRawMutex, _> = Mutex::new(RefCell::new(spi0));

    let dis_spi =
        SpiDeviceWithConfig::new(&spi0_bus, Output::new(dis_cs, Level::High), spi0_config);

    let pd_header = Header(0)
        .with_message_type_raw(DataMessageType::VendorDefined as u8)
        .with_num_objects(5) // 5 VDOs, vdm header, id header, cert, product, UFP product type
        .with_port_data_role(usb_pd::DataRole::Ufp)
        .with_port_power_role(usb_pd::PowerRole::Sink)
        .with_spec_revision(SpecificationRevision::R3_0);

    let vdm_header_vdo = pdo::VDMHeader::Structured(
        pdo::VDMHeaderStructured(0)
            .with_command(pdo::VDMCommand::DiscoverIdentity)
            .with_command_type(pdo::VDMCommandType::ResponderACK)
            .with_object_position(0) // 0 Must be used for descover identity
            .with_standard_or_vid(0xff00) // PD SID must be used with descover identity
            .with_vdm_type(pdo::VDMType::Structured)
            .with_vdm_version_major(VDMVersionMajor::Version2x.into())
            .with_vdm_version_minor(VDMVersionMinor::Version21.into()),
    );

    let id_header_vdo = pdo::VDMIdentityHeader(0)
        .with_host_data(false)
        .with_device_data(true)
        .with_product_type_ufp(pdo::SOPProductTypeUFP::PDUSBPeripheral)
        .with_product_type_dfp(pdo::SOPProductTypeDFP::NotDFP)
        .with_vid(0xc0ed);

    let cert_vdo = CertStatVDO(0x55aaaa55);

    let product_vdo = ProductVDO(0).with_pid(0xc0ed).with_bcd_device(0x0100);

    let product_type_vdo = UFPTypeVDO(0)
        .with_usb_highest_speed(USBHighestSpeed::USB20Only as u8)
        .with_device_capability(0x1)
        .with_version(UFPVDOVersion::Version1_3 as u8);

    // let mut VDOs = heapless::Vec::<u32, 5>::new();

    // VDOs.push(vdm_header_vdo.into()).unwrap_or_default();
    // VDOs.push(id_header_vdo.into()).unwrap_or_default();
    // VDOs.push(cert_vdo.into()).unwrap_or_default();
    // VDOs.push(product_vdo.into()).unwrap_or_default();
    // VDOs.push(product_type_vdo.into()).unwrap_or_default();

    loop {
        Timer::after_millis(100).await;
        info!(
            "PDO State:\n\tAccepted: {:?}\n\tReady: {:?}\n\tRequested: {:?}\n\tRejected: {:?}",
            PDO_LIST.lock(|pdos| pdos.borrow().accepted),
            PDO_LIST.lock(|pdos| pdos.borrow().ready),
            PDO_LIST.lock(|pdos| pdos.borrow().requested),
            PDO_LIST.lock(|pdos| pdos.borrow().rejected),
        );
    }
}

#[embassy_executor::task]
async fn pd_task(mut pd: PDSink) -> ! {
    loop {
        let now = Instant::now();
        let event = pd.poll(now);
        if let Some(event) = event {
            match event {
                Event::ProtocolChanged => {
                    info!("Protocol changed");
                }
                Event::SourceCapabilitiesChanged(caps) => {
                    info!("Source capabilities changed");
                    PDO_LIST.lock(|pdos| {
                        pdos.borrow_mut().pdos.clear();
                        pdos.borrow_mut().accepted = None;
                        pdos.borrow_mut().ready = None;
                        pdos.borrow_mut().requested = None;
                        pdos.borrow_mut().rejected = None;
                    });

                    let (index, supply) = caps
                        .iter()
                        .enumerate()
                        .filter_map(|(i, cap)| {
                            if let PowerDataObject::FixedSupply(supply) = cap {
                                debug!(
                                    "supply @ {}: {}mV {}mA",
                                    i,
                                    supply.voltage() * 50,
                                    supply.max_current() * 10
                                );

                                PDO_LIST.lock(|pdos| {
                                    pdos.borrow_mut().pdos.push(*cap).unwrap_or_default();
                                });

                                Some((i, supply))
                            } else {
                                warn!("skipping @ {}: {:?}", i, *cap);
                                None
                            }
                        })
                        .max_by(|(_, x), (_, y)| x.voltage().cmp(&y.voltage()))
                        .unwrap();

                    PDO_LIST.lock(|pdos| {
                        pdos.borrow_mut().requested = Some(index);
                    });

                    pd.request(Request::RequestPower {
                        index,
                        current: supply.max_current() * 10,
                    })
                }
                Event::PowerAccepted => {
                    info!("Power accepted");
                    PDO_LIST.lock(|pdos| {
                        let requested = pdos.borrow().requested.expect("No request made!");
                        pdos.borrow_mut().accepted = Some(requested);
                        pdos.borrow_mut().rejected = None;
                        pdos.borrow_mut().ready = None;
                        pdos.borrow_mut().requested = None;
                    });
                }
                Event::PowerRejected => {
                    info!("Power rejected");
                    PDO_LIST.lock(|pdos| {
                        let requested = pdos.borrow().requested.expect("No request made!");
                        pdos.borrow_mut().rejected = Some(requested);
                        pdos.borrow_mut().accepted = None;
                        pdos.borrow_mut().ready = None;
                        pdos.borrow_mut().requested = None;
                    });
                }
                Event::PowerReady => {
                    info!("Power ready");
                    PDO_LIST.lock(|pdos| {
                        let accepted = pdos.borrow().accepted.expect("No accepted request!");
                        pdos.borrow_mut().ready = Some(accepted);
                    });
                }
                Event::VDMReceived((hdr, data)) => {
                    match hdr {
                        VDMHeader::Structured(hdr) => {
                            info!(
                                "VDM structured\n\tCommand: {:?}\n\tCommand Type: {:?}\n\tPosition: {:?}\n\tData: {:X}",
                                hdr.command(),
                                hdr.command_type(),
                                hdr.object_position(),
                                data
                            );
                            match hdr.command_type() {
                                pdo::VDMCommandType::InitiatorREQ => {
                                    match hdr.command() {
                                        pdo::VDMCommand::DiscoverIdentity => {
                                            info!("VDM Discover Identity");
                                            // let di = VDMIdentityHeader(data[0]);
                                            // info!("VDM Discover Identity:\n\tVid: {:x}\n\tUFP: {:x}\n\tDFP: {:x}", di.vid(), di.product_type_ufp(), di.product_type_dfp());
                                        }
                                        pdo::VDMCommand::DiscoverSVIDS => {}
                                        pdo::VDMCommand::DiscoverModes => {}
                                        pdo::VDMCommand::EnterMode => {}
                                        pdo::VDMCommand::ExitMode => {}
                                        pdo::VDMCommand::Attention => {}
                                        pdo::VDMCommand::DisplayPortStatus => {}
                                        pdo::VDMCommand::DisplayPortConfig => {}
                                    }
                                }
                                _ => {
                                    panic!(
                                        "Unhandled VDM command type {:?} in RX",
                                        hdr.command_type()
                                    );
                                }
                            }
                        }
                        VDMHeader::Unstructured(hdr) => {
                            info!("VDM unstructured {:?}", hdr.data());
                        }
                    }
                }
            }
        }
        //        Timer::after_millis(1000).await;
        //        info!("PD task {}", now.as_millis());
        Timer::after_micros(100).await;
    }
}
