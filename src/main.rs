//! Template

#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::i2c::{self, Config};
use embassy_time::{Duration, Instant, Timer};
use embedded_hal_1::i2c::I2c;

use usb_pd::{
    pdo::PowerDataObject,
    sink::{Event, Request, Sink},
};

use {defmt_rtt as _, panic_probe as _};

type Fusb = fusb302b::Fusb302b<i2c::I2c<'static, embassy_rp::peripherals::I2C0, i2c::Blocking>>;
type PDSink = Sink<Fusb>;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    info!("Hello World!");

    let i2c0_sda = p.PIN_0;
    let i2c0_scl = p.PIN_1;

    let i2c0 = i2c::I2c::new_blocking(p.I2C0, i2c0_scl, i2c0_sda, Config::default());
    let fusb = fusb302b::Fusb302b::new(i2c0);
    let mut sink = Sink::new(fusb);

    sink.init();

    spawner.spawn(pd_task(sink)).unwrap();

    loop {
        Timer::after_millis(1000).await;
        info!("Hello World!");
    }
}

#[embassy_executor::task]
async fn pd_task(mut pd: PDSink) -> ! {
    loop {
        let now = Instant::now();
        let event = pd.poll(now);
        if let Some(event) = event {
            match event {
                Event::ProtocolChanged => {}
                Event::SourceCapabilitiesChanged(caps) => {
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
                                Some((i, supply))
                            } else {
                                None
                            }
                        })
                        .min_by(|(_, x), (_, y)| x.voltage().cmp(&y.voltage()))
                        .unwrap();

                    pd.request(Request::RequestPower {
                        index,
                        current: supply.max_current() * 10,
                    })
                }
                Event::PowerAccepted => {}
                Event::PowerRejected => {}
                Event::PowerReady => {}
            }
        }
        //        Timer::after_millis(1000).await;
        //        info!("PD task {}", now.as_millis());
        Timer::after_micros(100).await;
    }
}
