//! This example shows how async gpio can be used with a RP2040.
//!
//! The LED on the RP Pico W board is connected differently. See wifi_blinky.rs.

#![no_std]
#![no_main]

use core::panic;

use defmt::*;
use embassy_executor::{Executor, Spawner};
use embassy_futures::{join::join, select::select};
use embassy_rp::{
    adc::{self, Adc, Async},
    bind_interrupts, gpio,
    multicore::{Stack, spawn_core1},
    peripherals::USB,
    usb::{Driver, Instance, InterruptHandler},
};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{Channel, Sender},
};
use embassy_time::Timer;
use embassy_usb::{
    Builder, Config,
    class::cdc_acm::{CdcAcmClass, State},
    driver::EndpointError,
};
use gpio::{Input, Level, Output, Pull};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
    ADC_IRQ_FIFO => adc::InterruptHandler;
});

#[embassy_executor::task]
async fn logger_task(driver: Driver<'static, USB>) {
    embassy_usb_logger::run!(1024, log::LevelFilter::Debug, driver);
}

/// It requires an external signal to be manually triggered on PIN 16. For
/// example, this could be accomplished using an external power source with a
/// button so that it is possible to toggle the signal from low to high.
///
/// This example will begin with turning on the LED on the board and wait for a
/// high signal on PIN 16. Once the high event/signal occurs the program will
/// continue and turn off the LED, and then wait for 2 seconds before completing
/// the loop and starting over again.
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // Set up the ADC
    static ADC: StaticCell<Adc<'static, Async>> = StaticCell::new();
    let adc = ADC.init(Adc::new(p.ADC, Irqs, adc::Config::default()));
    // Set up the temperature sensor
    static TEMP_SENSOR: StaticCell<adc::Channel<'static>> = StaticCell::new();
    let temp_sensor = TEMP_SENSOR.init(adc::Channel::new_temp_sensor(p.ADC_TEMP_SENSOR));

    // Create the driver, from the HAL.
    let driver = Driver::new(p.USB, Irqs);

    // Create embassy-usb Config
    let mut config = Config::new(0xc0de, 0xcafe);
    config.manufacturer = Some("Embassy");
    config.product = Some("USB-serial example");
    config.serial_number = Some("12345678");
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    // Create embassy-usb DeviceBuilder using the driver and config.
    // It needs some buffers for building the descriptors.
    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];

    let mut state = State::new();
    let mut logger_state = State::new();

    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [], // no msos descriptors
        &mut control_buf,
    );

    // Create classes on the builder.
    let mut class = CdcAcmClass::new(&mut builder, &mut state, 64);

    // Create a class for the logger
    let logger_class = CdcAcmClass::new(&mut builder, &mut logger_state, 64);

    // Creates the logger and returns the logger future
    // Note: You'll need to use log::info! afterwards instead of info! for this to work (this also applies to all the other log::* macros)
    let log_fut = embassy_usb_logger::with_class!(1024, log::LevelFilter::Info, logger_class);

    // Build the builder.
    let mut usb = builder.build();

    // Run the USB device.
    let usb_fut = usb.run();

    // Channel
    static CHANNEL: StaticCell<Channel<CriticalSectionRawMutex, Result<f32, adc::Error>, 1>> =
        StaticCell::new();
    let channel = CHANNEL.init(Channel::new());
    let sender = channel.sender();
    let receiver = channel.receiver();

    // LED controller
    let led_fut = async {
        let mut led = Output::new(p.PIN_25, Level::Low);
        let mut inp = Input::new(p.PIN_16, Pull::Down);

        loop {
            log::info!("Turn on LED");
            led.set_high();

            select(
                async {
                    inp.wait_for_high().await;
                    log::info!("User!");
                },
                async {
                    let val = receiver.receive().await;
                    match val {
                        Ok(val) => log::info!("Temperature: {:.2} C.", val),
                        Err(e) => log::error!("Error receiving temperature: {:?}", e),
                    }
                },
            )
            .await;
            Timer::after_secs(1).await;
            log::info!("Turn off LED");
            led.set_low();
            Timer::after_secs(1).await;
        }
    };

    // Do stuff with the class!
    let echo_fut = async {
        loop {
            class.wait_connection().await;
            log::info!("Connected");
            let _ = echo(&mut class).await;
            log::info!("Disconnected");
        }
    };

    // Spawn core 1
    static CORE1_STACK: StaticCell<Stack<512>> = StaticCell::new();
    static CORE1_EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let stack = CORE1_STACK.init(Stack::new());
    spawn_core1(p.CORE1, stack, move || {
        let exec = CORE1_EXECUTOR.init(Executor::new());
        exec.run(|spawner| {
            if let Err(e) = spawner.spawn(core1_ctr(sender, adc, temp_sensor)) {
                log::error!("Could not spawn core 1: {e:?}");
            }
        })
    });

    // Spawn other tasks
    join(usb_fut, join(echo_fut, join(led_fut, log_fut))).await;
}

struct Disconnected {}

impl From<EndpointError> for Disconnected {
    fn from(val: EndpointError) -> Self {
        match val {
            EndpointError::BufferOverflow => panic!("Buffer overflow"),
            EndpointError::Disabled => Disconnected {},
        }
    }
}

async fn echo<'d, T: Instance + 'd>(
    class: &mut CdcAcmClass<'d, Driver<'d, T>>,
) -> Result<(), Disconnected> {
    let mut buf = [0; 64];
    loop {
        let n = class.read_packet(&mut buf).await?;
        let data = &buf[..n];
        info!("data: {:x}", data);
        class.write_packet(data).await?;
    }
}

#[embassy_executor::task]
async fn core1_ctr(
    sender: Sender<'static, CriticalSectionRawMutex, Result<f32, adc::Error>, 1>,
    adc: &'static mut Adc<'static, Async>,
    temp_sensor: &'static mut adc::Channel<'static>,
) {
    loop {
        // Read the temperature sensor
        let temp = adc.read(temp_sensor).await;
        sender
            .send(temp.map(|temp| {
                let volt = (temp as f32 / 4095.0) * 3.3;
                27.0 - ((volt - 0.706) / 0.001721)
            }))
            .await;
    }
}
