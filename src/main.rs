use core::time::Duration;

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::prelude::*;
use esp_idf_hal::rmt::{config::TransmitConfig, FixedLengthSignal, PinState, Pulse, TxRmtDriver};

fn main() {
    // Required by esp-idf-sys for link patches.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();
    let led = peripherals.pins.gpio48;
    let channel = peripherals.rmt.channel0;
    let config = TransmitConfig::new().clock_divider(1);
    let mut tx = TxRmtDriver::new(channel, led, &config).unwrap();

    let mut on = true;
    loop {
        let color = if on { Rgb::new(16, 16, 16) } else { Rgb::new(0, 0, 0) };
        neopixel(color, &mut tx);
        on = !on;
        FreeRtos::delay_ms(500);
    }
}

fn neopixel(rgb: Rgb, tx: &mut TxRmtDriver) {
    let color: u32 = rgb.into();
    let ticks_hz = tx.counter_clock().unwrap();
    let (t0h, t0l, t1h, t1l) = (
        Pulse::new_with_duration(ticks_hz, PinState::High, &Duration::from_nanos(350)).unwrap(),
        Pulse::new_with_duration(ticks_hz, PinState::Low, &Duration::from_nanos(800)).unwrap(),
        Pulse::new_with_duration(ticks_hz, PinState::High, &Duration::from_nanos(700)).unwrap(),
        Pulse::new_with_duration(ticks_hz, PinState::Low, &Duration::from_nanos(600)).unwrap(),
    );
    let mut signal = FixedLengthSignal::<24>::new();
    for i in (0..24).rev() {
        let bit: bool = (1_u32 << i) & color != 0;
        let (high_pulse, low_pulse) = if bit { (t1h, t1l) } else { (t0h, t0l) };
        signal.set(23 - i as usize, &(high_pulse, low_pulse)).unwrap();
    }
    let _ = tx.start_blocking(&signal);
}

struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

impl Rgb {
    fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl From<Rgb> for u32 {
    fn from(rgb: Rgb) -> Self {
        ((rgb.g as u32) << 16) | ((rgb.r as u32) << 8) | rgb.b as u32
    }
}
