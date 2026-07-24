use std::time::{Duration, Instant, SystemTime};

use heapless::String;

#[derive(Clone, PartialEq, Eq)]
pub struct UsageState {
    pub plan: String<16>,
    pub five_hour: WindowUsage,
    pub weekly: WindowUsage,
    pub credits: Option<Credits>,
    pub connectivity: Connectivity,
    pub last_updated: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct WindowUsage {
    pub used_percent: u8,
    pub resets_at: SystemTime,
}

impl WindowUsage {
    pub fn new(used_percent: u8, resets_at: SystemTime) -> Self {
        Self { used_percent: used_percent.min(100), resets_at }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Credits { pub balance: String<24> }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Connectivity { Offline, Connecting, Online }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UsageError {
    Unavailable,
    Unauthorized,
    InvalidResponse,
    Transport,
}

pub trait UsageProvider {
    fn refresh(&mut self) -> Result<UsageState, UsageError>;
}

pub struct MockUsageProvider;

impl MockUsageProvider {
    pub const fn new() -> Self { Self }
}

impl Default for MockUsageProvider {
    fn default() -> Self { Self::new() }
}

impl UsageProvider for MockUsageProvider {
    fn refresh(&mut self) -> Result<UsageState, UsageError> {
        let now = Instant::now();
        let reset = SystemTime::now() + Duration::from_secs(2 * 60 * 60 + 14 * 60);
        let mut plan = String::<16>::new();
        let _ = plan.push_str("Pro");
        Ok(UsageState {
            plan,
            five_hour: WindowUsage::new(76, reset),
            weekly: WindowUsage::new(48, reset),
            credits: None,
            connectivity: Connectivity::Online,
            last_updated: now.checked_sub(Duration::from_secs(12)).unwrap_or(now),
        })
    }
}
