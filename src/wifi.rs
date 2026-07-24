use anyhow::{anyhow, Result};
use std::time::{Duration, Instant};

use crate::domain::Connectivity;
use crate::storage::{SettingsStore, WifiCredentials};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WifiError { InvalidCredentials, AssociationFailed }

impl std::fmt::Display for WifiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self { Self::InvalidCredentials => "invalid Wi-Fi credentials", Self::AssociationFailed => "Wi-Fi association failed" })
    }
}
impl std::error::Error for WifiError {}

const RETRIES: [Duration; 4] = [Duration::from_secs(1), Duration::from_secs(2), Duration::from_secs(4), Duration::from_secs(8)];

/// Provider-neutral Wi-Fi state machine. ESP-IDF handles are deliberately kept
/// out of this public boundary; the adapter reports association outcomes here.
pub struct WiFiManager {
    state: Connectivity,
    next_attempt: Option<Instant>,
    retry_index: usize,
    last_attempt: Option<Instant>,
    association_ok: bool,
    saved_ssid_len: Option<usize>,
}

impl WiFiManager {
    pub fn new() -> Self { Self { state: Connectivity::Offline, next_attempt: None, retry_index: 0, last_attempt: None, association_ok: true, saved_ssid_len: None } }
    pub fn state(&self) -> Connectivity { self.state }
    /// Test/adapter hook: the ESP-IDF event handler sets this on association result.
    pub fn set_association_result(&mut self, success: bool) { self.association_ok = success; }
    pub fn connect_saved(&mut self, settings: &SettingsStore) -> Result<Connectivity, WifiError> {
        let Some(credentials) = settings.load().wifi else { self.state = Connectivity::Offline; return Ok(self.state); };
        self.saved_ssid_len = Some(credentials.ssid_len());
        self.begin_attempt(Instant::now());
        Ok(self.state)
    }
    pub fn save_and_connect(&mut self, settings: &mut SettingsStore, ssid: &str, password: &str) -> Result<Connectivity, WifiError> {
        let credentials = WifiCredentials::new(ssid, password).map_err(|_| WifiError::InvalidCredentials)?;
        self.saved_ssid_len = Some(credentials.ssid_len());
        self.begin_attempt(Instant::now());
        if !self.association_ok { self.state = Connectivity::Offline; return Err(WifiError::AssociationFailed); }
        settings.save_wifi(credentials).map_err(|_| WifiError::AssociationFailed)?;
        self.state = Connectivity::Online;
        self.retry_index = 0;
        self.next_attempt = None;
        Ok(self.state)
    }
    pub fn tick(&mut self, now: Instant) -> Connectivity {
        if self.state == Connectivity::Online { return self.state; }
        if self.next_attempt.is_some_and(|at| now < at) { return self.state; }
        if self.saved_ssid_len.is_none() { self.state = Connectivity::Offline; return self.state; }
        self.begin_attempt(now);
        if self.association_ok {
            self.state = Connectivity::Online;
            self.retry_index = 0;
            self.next_attempt = None;
        } else {
            self.state = Connectivity::Offline;
            let delay = RETRIES.get(self.retry_index).copied().unwrap_or(Duration::from_secs(30));
            self.retry_index = self.retry_index.saturating_add(1);
            self.next_attempt = Some(now + delay);
        }
        self.state
    }
    fn begin_attempt(&mut self, now: Instant) { self.state = Connectivity::Connecting; self.last_attempt = Some(now); }
    pub fn retry_delay(&self) -> Option<Duration> { self.next_attempt.and_then(|_| RETRIES.get(self.retry_index.saturating_sub(1)).copied().or(Some(Duration::from_secs(30)))) }
    pub fn saved_ssid_len(&self) -> Option<usize> { self.saved_ssid_len }
}

#[cfg(target_os = "espidf")]
mod esp_idf_adapter {
    use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
    #[allow(dead_code)] type Wifi<'a> = BlockingWifi<EspWifi<'a>>;
}
