mod auth;
mod board;
mod display;
mod domain;
mod screens;
mod storage;
mod touch;
mod wifi;
mod secrets;
mod providers;

use anyhow::Result;
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::spi::Operation;
use esp_idf_hal::gpio::{PinDriver, Pull};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::spi::{config, SpiDeviceDriver, SpiDriverConfig};
use esp_idf_hal::units::FromValueType;

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    let peripherals = Peripherals::take()?;

    let mut dc = PinDriver::output(peripherals.pins.gpio2)?;
    let mut touch_irq = PinDriver::input(peripherals.pins.gpio36, Pull::Floating)?;
    let mut backlight = PinDriver::output(peripherals.pins.gpio21)?;

    let lcd_config = config::Config::new().baudrate(8.MHz().into());
    let mut lcd = SpiDeviceDriver::new_single(
        peripherals.spi2,
        peripherals.pins.gpio14,
        peripherals.pins.gpio13,
        Some(peripherals.pins.gpio12),
        Some(peripherals.pins.gpio15),
        &SpiDriverConfig::new(),
        &lcd_config,
    )?;
    let touch_config = config::Config::new().baudrate(2.MHz().into());
    let mut touch = SpiDeviceDriver::new_single(
        peripherals.spi3,
        peripherals.pins.gpio25,
        peripherals.pins.gpio32,
        Some(peripherals.pins.gpio39),
        Some(peripherals.pins.gpio33),
        &SpiDriverConfig::new(),
        &touch_config,
    )?;

    backlight.set_high()?;
    FreeRtos::delay_ms(120);
    log::info!("diagnostic firmware starting");
    log::info!("candidate ESP32-2432S028 LCD and XPT2046 SPI buses initialized");

    // Minimal ILI9341 initialization and six visible horizontal color bands.
    dc.set_low()?;
    lcd.write(&[0x11])?; // sleep out
    dc.set_high()?;
    FreeRtos::delay_ms(120);
    dc.set_low()?;
    lcd.write(&[0x3A])?; // 16-bit RGB565
    dc.set_high()?;
    lcd.write(&[0x55])?;
    dc.set_low()?;
    lcd.write(&[0x36])?; // portrait orientation
    dc.set_high()?;
    lcd.write(&[0x48])?;
    dc.set_low()?;
    lcd.write(&[0x29])?; // display on
    dc.set_high()?;
    FreeRtos::delay_ms(20);

    dc.set_low()?;
    lcd.write(&[0x2A])?;
    dc.set_high()?;
    lcd.write(&[0x00, 0x00, 0x01, 0x3F])?;
    dc.set_low()?;
    lcd.write(&[0x2B])?;
    dc.set_high()?;
    lcd.write(&[0x00, 0x00, 0x00, 0xEF])?;
    dc.set_low()?;
    lcd.write(&[0x2C])?;
    dc.set_high()?;
    for color in [0xF800u16, 0x07E0, 0x001F, 0xFFE0, 0xF81F, 0x07FF] {
        let mut row = [0u8; 640];
        for pixel in row.chunks_exact_mut(2) {
            pixel[0] = (color >> 8) as u8;
            pixel[1] = color as u8;
        }
        for _ in 0..40 {
            lcd.write(&row)?;
        }
    }
    log::info!("ILI9341 init and color bands sent");

    for command in [0x04u8, 0xD3] {
        dc.set_low()?;
        lcd.write(&[command])?;
        dc.set_high()?;
        let mut response = [0u8; 4];
        lcd.read(&mut response)?;
        log::info!("LCD command 0x{command:02x} response={response:02x?}");
    }

    for (index, color) in [0xF800u16, 0x07E0, 0x001F, 0xFFE0, 0xF81F, 0x07FF].iter().enumerate() {
        dc.set_low()?;
        lcd.write(&[0x2C])?;
        dc.set_high()?;
        let bytes = [(*color >> 8) as u8, *color as u8];
        lcd.write(&bytes)?;
        log::info!("color bar {index}: RGB565 0x{color:04x}");
    }

    log::info!("diagnostic complete; touch the panel corners while serial capture runs");
    let mut sample = 0u32;
    loop {
        for command in [0x90u8, 0xD0] {
            let mut response = [0u8; 3];
            touch.transaction(&mut [Operation::Write(&[command, 0]), Operation::Read(&mut response)])?;
            log::info!(
                "touch sample {sample}: command=0x{command:02x} irq_low={} raw={response:02x?}",
                touch_irq.is_low()
            );
            sample = sample.wrapping_add(1);
        }
        FreeRtos::delay_ms(500);
    }
}
