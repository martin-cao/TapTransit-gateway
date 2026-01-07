use std::thread;
use std::time::{Duration, Instant};

use esp_idf_hal::gpio::OutputPin;
use esp_idf_hal::rmt::{config::TransmitConfig, FixedLengthSignal, PinState, Pulse, TxRmtDriver};
use esp_idf_hal::sys::EspError;
use esp_idf_hal::{peripheral::Peripheral, rmt::RmtChannel};
use smart_leds::{RGB8, SmartLedsWrite};

use crate::model::PassengerTone;
use crate::state::GatewayState;
use std::sync::{Arc, Mutex};

// 亮度缩放（约 30%）。
const BRIGHTNESS_SCALE: u8 = 77;

/// WS2812 智能灯封装（通过 RMT 发送）。
pub struct SmartLed<'d> {
    tx: TxRmtDriver<'d>,
}

impl<'d> SmartLed<'d> {
    /// 初始化 RMT 发送器。
    pub fn new<C, P, Ch, Pin>(channel: C, pin: P) -> Result<Self, EspError>
    where
        C: Peripheral<P = Ch> + 'd,
        P: Peripheral<P = Pin> + 'd,
        Ch: RmtChannel,
        Pin: OutputPin,
    {
        let config = TransmitConfig::new().clock_divider(1);
        let tx = TxRmtDriver::new(channel, pin, &config)?;
        Ok(Self { tx })
    }

    /// 设置单色显示。
    pub fn set_color(&mut self, color: RGB8) -> Result<(), EspError> {
        self.write([color].into_iter())
    }

    /// 亮度缩放，降低刺眼程度。
    fn apply_brightness(color: RGB8) -> RGB8 {
        let scale = BRIGHTNESS_SCALE as u16;
        let apply = |v| ((v as u16 * scale) / 255) as u8;
        RGB8 {
            r: apply(color.r),
            g: apply(color.g),
            b: apply(color.b),
        }
    }

    /// 生成 GRB 24bit 脉冲序列。
    fn render_signal(&self, color: RGB8) -> Result<FixedLengthSignal<24>, EspError> {
        let color = Self::apply_brightness(color);
        let grb: u32 = ((color.g as u32) << 16) | ((color.r as u32) << 8) | color.b as u32;
        let ticks_hz = self.tx.counter_clock()?;
        let (t0h, t0l, t1h, t1l) = (
            Pulse::new_with_duration(ticks_hz, PinState::High, &Duration::from_nanos(350))?,
            Pulse::new_with_duration(ticks_hz, PinState::Low, &Duration::from_nanos(800))?,
            Pulse::new_with_duration(ticks_hz, PinState::High, &Duration::from_nanos(700))?,
            Pulse::new_with_duration(ticks_hz, PinState::Low, &Duration::from_nanos(600))?,
        );
        let mut signal = FixedLengthSignal::<24>::new();
        for i in (0..24).rev() {
            let bit = (grb & (1 << i)) != 0;
            let (hi, lo) = if bit { (t1h, t1l) } else { (t0h, t0l) };
            signal.set(23 - i as usize, &(hi, lo))?;
        }
        Ok(signal)
    }
}

impl SmartLedsWrite for SmartLed<'_> {
    type Color = RGB8;
    type Error = EspError;

    fn write<T, I>(&mut self, iterator: T) -> Result<(), Self::Error>
    where
        T: IntoIterator<Item = I>,
        I: Into<Self::Color>,
    {
        let mut iter = iterator.into_iter();
        let color = iter.next().map(Into::into).unwrap_or(RGB8::default());
        let signal = self.render_signal(color)?;
        self.tx.start_blocking(&signal)?;
        Ok(())
    }
}

/// 启动灯带任务：根据刷卡状态闪烁颜色。
pub fn spawn_led_task<C, P, Ch, Pin>(channel: C, pin: P, state: Arc<Mutex<GatewayState>>)
where
    C: Peripheral<P = Ch> + Send + 'static,
    P: Peripheral<P = Pin> + Send + 'static,
    Ch: RmtChannel + Send + 'static,
    Pin: OutputPin + Send + 'static,
{
    thread::spawn(move || {
        let mut led = match SmartLed::new(channel, pin) {
            Ok(led) => led,
            Err(err) => {
                log::warn!("Smart LED init failed: {:?}", err);
                return;
            }
        };
        let _ = led.set_color(RGB8::default());
        let mut last_nonce: u32 = 0;
        let mut last_tone = PassengerTone::Normal;
        let mut led_on = false;
        let mut display_until: Option<Instant> = None;
        loop {
            let mut next_tone = None;
            if let Ok(state) = state.lock() {
                let current_tone = state.last_passenger_tone;
                // 新刷卡触发或提示音改变则更新灯色
                if state.last_tap_nonce != last_nonce {
                    last_nonce = state.last_tap_nonce;
                    last_tone = current_tone;
                    next_tone = Some(last_tone);
                } else if led_on && current_tone != last_tone {
                    last_tone = current_tone;
                    next_tone = Some(last_tone);
                }
            }
            if let Some(tone) = next_tone {
                let color = tone_color(tone);
                if let Err(err) = led.set_color(color) {
                    log::warn!("Smart LED update failed: {:?}", err);
                }
                display_until = Some(Instant::now() + Duration::from_secs(1));
                led_on = true;
            }
            if led_on {
                if let Some(until) = display_until {
                    if Instant::now() >= until {
                        if let Err(err) = led.set_color(RGB8::default()) {
                            log::warn!("Smart LED update failed: {:?}", err);
                        }
                        led_on = false;
                    }
                }
            }
            thread::sleep(Duration::from_millis(150));
        }
    });
}

/// 将提示音色映射到 LED 颜色。
fn tone_color(tone: PassengerTone) -> RGB8 {
    match tone {
        PassengerTone::Normal => RGB8 { r: 0, g: 0, b: 255 },
        PassengerTone::Error => RGB8 { r: 255, g: 0, b: 0 },
        PassengerTone::Student => RGB8 { r: 0, g: 255, b: 0 },
        PassengerTone::Elder => RGB8 { r: 255, g: 255, b: 0 },
        PassengerTone::Disabled => RGB8 { r: 0, g: 255, b: 255 },
    }
}
