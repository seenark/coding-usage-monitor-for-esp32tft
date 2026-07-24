//! Direct-display boundary. Hardware construction is intentionally deferred
//! until the board probe validates [`crate::board::CANDIDATE`].

use anyhow::{bail, Result};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::{Point, RgbColor, Size};
use embedded_graphics::primitives::Rectangle;

pub const DISPLAY_SIZE: Size = Size::new(320, 240);

/// Drawing target supplied by the eventual ILI9341 backend.
/// It contains no framebuffer; renderers issue commands directly to the panel.
#[derive(Debug, Default)]
pub struct Display {
    origin: Point,
    extent: Size,
}

impl Display {
    pub fn new(region: Rectangle) -> Self { Self { origin: region.top_left, extent: region.size } }
    pub fn origin(&self) -> Point { self.origin }
    pub fn size(&self) -> Size { self.extent }
}

#[derive(Debug)]
pub struct DisplayDriver {
    brightness_percent: u8,
    last_region: Option<Rectangle>,
    color: Rgb565,
}

impl Default for DisplayDriver {
    fn default() -> Self { Self::new() }
}

impl DisplayDriver {
    pub fn new() -> Self {
        Self { brightness_percent: 60, last_region: None, color: Rgb565::BLACK }
    }

    pub fn brightness(&self) -> u8 { self.brightness_percent }

    /// Clamp persisted/user brightness to the safe, non-zero PWM range.
    pub fn set_brightness(&mut self, percent: u8) -> Result<()> {
        self.brightness_percent = percent.clamp(1, 100);
        Ok(())
    }

    pub fn draw_region<F>(&mut self, region: Rectangle, draw: F) -> Result<()>
    where F: FnOnce(&mut Display) -> Result<()> {
        if region.size.width == 0 || region.size.height == 0 { bail!("empty display region") }
        let bounds = Rectangle::new(Point::zero(), DISPLAY_SIZE);
        if !bounds.contains(region.top_left)
            || region.bottom_right().map_or(true, |point| !bounds.contains(point))
        {
            bail!("display region outside 320x240 surface")
        }
        let mut surface = Display::new(region);
        draw(&mut surface)?;
        self.last_region = Some(region);
        Ok(())
    }

    pub fn clear(&mut self, color: Rgb565) -> Result<()> {
        self.color = color;
        self.last_region = Some(Rectangle::new(Point::zero(), DISPLAY_SIZE));
        Ok(())
    }

    pub fn last_region(&self) -> Option<Rectangle> { self.last_region }
}
