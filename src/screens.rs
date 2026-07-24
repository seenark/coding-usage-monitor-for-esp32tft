use std::time::{Duration, SystemTime};

use embedded_graphics::{geometry::Point, primitives::Rectangle};
use heapless::{String, Vec};
use serde::{Deserialize, Serialize};

use crate::domain::{Connectivity, UsageState};
use crate::storage::SettingsV1;
use crate::touch::TouchEvent;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ScreenId { Dashboard, Gauges, Terminal }

#[derive(Clone, PartialEq, Eq)]
pub struct ScreenFrame {
    pub screen: ScreenId,
    pub title: String<24>,
    pub lines: Vec<String<64>, 12>,
}

pub struct ScreenManager {
    current: ScreenId,
    dirty: Vec<Rectangle, 16>,
}

impl ScreenManager {
    pub fn new(default_screen: ScreenId) -> Self {
        let mut this = Self { current: default_screen, dirty: Vec::new() };
        this.invalidate_full();
        this
    }

    pub fn current(&self) -> ScreenId { self.current }

    pub fn handle_event(&mut self, event: TouchEvent) -> Option<ScreenId> {
        let next = match event {
            TouchEvent::SwipeLeft => Some(self.current.next()),
            TouchEvent::SwipeRight => Some(self.current.previous()),
            TouchEvent::LongPress(_) => None, // settings overlay is owned by the caller
            _ => return None,
        };
        if let Some(screen) = next {
            self.current = screen;
            self.invalidate_full();
        }
        next
    }

    pub fn invalidate(&mut self, region: Rectangle) { let _ = self.dirty.push(region); }

    pub fn invalidate_full(&mut self) {
        self.invalidate(Rectangle::new(Point::new(0, 0), embedded_graphics::geometry::Size::new(320, 240)));
    }

    pub fn take_dirty_rectangles(&mut self) -> Vec<Rectangle, 16> {
        core::mem::take(&mut self.dirty)
    }

    pub fn render(&self, usage: &UsageState, _settings: &SettingsV1, now: SystemTime) -> ScreenFrame {
        match self.current {
            ScreenId::Dashboard => dashboard(usage, now),
            ScreenId::Gauges => gauges(usage, now),
            ScreenId::Terminal => terminal(usage, now),
        }
    }
}

impl ScreenId {
    pub fn next(self) -> Self { match self { Self::Dashboard => Self::Gauges, Self::Gauges => Self::Terminal, Self::Terminal => Self::Dashboard } }
    pub fn previous(self) -> Self { match self { Self::Dashboard => Self::Terminal, Self::Gauges => Self::Dashboard, Self::Terminal => Self::Gauges } }
}

fn text<const N: usize>(value: &str) -> String<N> { let mut out = String::new(); let _ = out.push_str(value); out }
fn age(usage: &UsageState) -> u64 { usage.last_updated.elapsed().as_secs() }
fn countdown(at: SystemTime, now: SystemTime) -> u64 { at.duration_since(now).unwrap_or(Duration::ZERO).as_secs() }

fn dashboard(u: &UsageState, _now: SystemTime) -> ScreenFrame {
    let mut lines = Vec::new();
    let _ = lines.push(text(&format!("5-hour  {}%", u.five_hour.used_percent)));
    let _ = lines.push(text(&format!("Weekly  {}%", u.weekly.used_percent)));
    let _ = lines.push(text("Reset countdown"));
    let _ = lines.push(text(if u.connectivity == Connectivity::Online { "Online" } else { "Offline" }));
    let _ = lines.push(text(&format!("Updated {} sec ago", age(u))));
    ScreenFrame { screen: ScreenId::Dashboard, title: text("Codex Usage"), lines }
}
fn gauges(u: &UsageState, now: SystemTime) -> ScreenFrame {
    let mut lines = Vec::new();
    let _ = lines.push(text(&format!("5-hour gauge {}%", u.five_hour.used_percent)));
    let _ = lines.push(text(&format!("Weekly gauge {}%", u.weekly.used_percent)));
    let _ = lines.push(text(&format!("Reset {} sec", countdown(u.five_hour.resets_at, now))));
    ScreenFrame { screen: ScreenId::Gauges, title: text("Gauges"), lines }
}
fn terminal(u: &UsageState, now: SystemTime) -> ScreenFrame {
    let mut lines = Vec::new();
    let _ = lines.push(text("CODEX USAGE"));
    let _ = lines.push(text(&format!("PLAN      {}", u.plan.as_str())));
    let _ = lines.push(text(&format!("5H    {}%", u.five_hour.used_percent)));
    let _ = lines.push(text(&format!("WEEK  {}%", u.weekly.used_percent)));
    let _ = lines.push(text(&format!("RESET {} SEC", countdown(u.five_hour.resets_at, now))));
    let _ = lines.push(text(&format!("SYNC {} SEC AGO", age(u))));
    ScreenFrame { screen: ScreenId::Terminal, title: text("Terminal"), lines }
}
