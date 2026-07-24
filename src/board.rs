//! Board configuration and hardware-validation boundary.
//!
//! The values below are the documented ESP32-2432S028 candidate mapping.  They
//! are deliberately marked pending until a physical probe confirms this board.

use embedded_graphics::prelude::Size;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerKind {
    Ili9341FamilyPendingProbe,
    Ili9341Validated,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorOrder {
    Rgb,
    Bgr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardPins {
    pub lcd_sclk: i32,
    pub lcd_mosi: i32,
    pub lcd_miso: i32,
    pub lcd_cs: i32,
    pub lcd_dc: i32,
    pub lcd_reset: Option<i32>,
    pub backlight: i32,
    pub touch_sclk: i32,
    pub touch_mosi: i32,
    pub touch_miso: i32,
    pub touch_cs: i32,
    pub touch_irq: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardConfig {
    pub pins: BoardPins,
    pub surface: Size,
    pub rotation: u8,
    pub color_order: ColorOrder,
    pub lcd_controller: ControllerKind,
    pub touch_controller: &'static str,
    pub runtime_validated: bool,
}

/// Candidate only: no connected board probe has been observed in this build.
pub const CANDIDATE: BoardConfig = BoardConfig {
    pins: BoardPins {
        lcd_sclk: 14, lcd_mosi: 13, lcd_miso: 12, lcd_cs: 15, lcd_dc: 2,
        lcd_reset: None, backlight: 21,
        touch_sclk: 25, touch_mosi: 32, touch_miso: 39, touch_cs: 33, touch_irq: 36,
    },
    surface: Size::new(320, 240),
    rotation: 1,
    color_order: ColorOrder::Rgb,
    lcd_controller: ControllerKind::Ili9341FamilyPendingProbe,
    touch_controller: "XPT2046 (pending runtime probe)",
    runtime_validated: false,
};

pub fn candidate() -> BoardConfig { CANDIDATE }
