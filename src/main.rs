//! Template

#![no_std]
#![no_main]

use core::{cell::RefCell, panic};

use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::i2c::{self, Config};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_time::{Instant, Timer};
//use embedded_hal_1::i2c::I2c;

use usb_pd::{
    pdo::{self, PowerDataObject, VDMHeader},
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

    let i2c0_sda = p.PIN_0;
    let i2c0_scl = p.PIN_1;

    let i2c0 = i2c::I2c::new_blocking(p.I2C0, i2c0_scl, i2c0_sda, Config::default());
    let fusb = fusb302b::Fusb302b::new(i2c0);
    let mut sink = Sink::new(fusb);

    sink.init();

    spawner.spawn(pd_task(sink)).unwrap();

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
                Event::VDM((hdr, data)) => {
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
