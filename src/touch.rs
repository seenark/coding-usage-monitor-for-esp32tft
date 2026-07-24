//! XPT2046 touch boundary and deterministic gesture reduction.

use anyhow::{bail, Result};
use embedded_graphics::prelude::Point;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchEvent { Down(Point), Move(Point), Up(Point), LongPress(Point), SwipeLeft, SwipeRight }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchCalibration { pub x_min: u16, pub x_max: u16, pub y_min: u16, pub y_max: u16, pub rotation: u8 }

impl TouchCalibration {
    pub fn from_samples(x: &mut [u16], y: &mut [u16], rotation: u8) -> Result<Self> {
        if x.is_empty() || x.len() != y.len() { bail!("calibration requires paired samples") }
        x.sort_unstable(); y.sort_unstable();
        let (xm, xx) = (*x.first().unwrap(), *x.last().unwrap());
        let (ym, yx) = (*y.first().unwrap(), *y.last().unwrap());
        if xx.saturating_sub(xm) < 100 || yx.saturating_sub(ym) < 100 { bail!("degenerate calibration span") }
        Ok(Self { x_min: xm, x_max: xx, y_min: ym, y_max: yx, rotation })
    }
    pub fn map(&self, raw_x: u16, raw_y: u16) -> Point {
        let x = ((raw_x.clamp(self.x_min, self.x_max) - self.x_min) as u32 * 319 / (self.x_max-self.x_min) as u32) as i32;
        let y = ((raw_y.clamp(self.y_min, self.y_max) - self.y_min) as u32 * 239 / (self.y_max-self.y_min) as u32) as i32;
        Point::new(x, y)
    }
}

#[derive(Debug)]
pub struct TouchManager {
    calibration: Option<TouchCalibration>,
    contact: Option<(Point, Instant, Point)>,
    pending: Option<TouchEvent>,
}

impl TouchManager {
    pub fn new(calibration: Option<TouchCalibration>) -> Self { Self { calibration, contact: None, pending: None } }
    pub fn calibration(&self) -> Option<TouchCalibration> { self.calibration }
    pub fn poll(&mut self, now: Instant) -> Result<Option<TouchEvent>> {
        if let Some(event) = self.pending.take() { return Ok(Some(event)); }
        if let Some((start, at, last)) = self.contact { if now.duration_since(at) >= Duration::from_millis(700) && (last.x-start.x).abs() <= 12 && (last.y-start.y).abs() <= 12 { self.contact = None; return Ok(Some(TouchEvent::LongPress(last))); } }
        Ok(None)
    }
    pub fn ingest(&mut self, event: TouchEvent, now: Instant) {
        match event { TouchEvent::Down(p) => { self.contact = Some((p, now, p)); }, TouchEvent::Move(p) => if let Some((s,t,_))=self.contact { self.contact=Some((s,t,p)); }, TouchEvent::Up(p) => { if let Some((s,_,_))=self.contact { let dx=p.x-s.x; let dy=(p.y-s.y).abs(); if dx.abs() >= 80 && dx.abs() > dy { self.pending=Some(if dx<0 {TouchEvent::SwipeLeft} else {TouchEvent::SwipeRight}); } else { self.pending=Some(TouchEvent::Up(p)); } self.contact=None; } }, _ => self.pending=Some(event) }
    }
}
