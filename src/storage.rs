use anyhow::{anyhow, Result};
use heapless::String;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

use crate::screens::ScreenId;
pub const SETTINGS_VERSION: u8 = 1;
pub const MAX_SSID_BYTES: usize = 32;
pub const MAX_PASSWORD_BYTES: usize = 63;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WifiCredentials {
    pub ssid: String<MAX_SSID_BYTES>,
    pub password: String<MAX_PASSWORD_BYTES>,
}

impl WifiCredentials {
    pub fn new(ssid: &str, password: &str) -> Result<Self> {
        let ssid = String::try_from(ssid).map_err(|_| anyhow!("SSID exceeds 32 UTF-8 bytes"))?;
        let password = String::try_from(password).map_err(|_| anyhow!("password exceeds 63 UTF-8 bytes"))?;
        if ssid.is_empty() { return Err(anyhow!("SSID must not be empty")); }
        Ok(Self { ssid, password })
    }
    pub fn ssid_len(&self) -> usize { self.ssid.len() }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct TouchCalibration {
    pub x_min: u16,
    pub x_max: u16,
    pub y_min: u16,
    pub y_max: u16,
    pub orientation: u8,
}

impl TouchCalibration {
    pub fn valid(&self) -> bool {
        self.x_max.saturating_sub(self.x_min) >= 100 && self.y_max.saturating_sub(self.y_min) >= 100
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingsV1 {
    pub version: u8,
    pub wifi: Option<WifiCredentials>,
    pub brightness_percent: u8,
    pub default_screen: ScreenId,
    pub touch_calibration: Option<TouchCalibration>,
}

impl Default for SettingsV1 {
    fn default() -> Self {
        Self { version: SETTINGS_VERSION, wifi: None, brightness_percent: 60, default_screen: ScreenId::Dashboard, touch_calibration: None }
    }
}

impl SettingsV1 {
    pub fn normalized(mut self) -> Self {
        self.version = SETTINGS_VERSION;
        self.brightness_percent = self.brightness_percent.clamp(1, 100);
        if self.touch_calibration.as_ref().is_some_and(|c| !c.valid()) { self.touch_calibration = None; }
        self
    }
}

/// Settings boundary. The in-memory value is also used by host-side compile checks;
/// the ESP-IDF application replaces the persistence backend without exposing NVS types.
pub struct SettingsStore { settings: SettingsV1 }

impl SettingsStore {
    pub fn new() -> Self { Self { settings: SettingsV1::default() } }
    pub fn load(&self) -> SettingsV1 { self.settings.clone() }
    pub fn save(&mut self, settings: &SettingsV1) -> Result<()> { self.settings = settings.clone().normalized(); Ok(()) }
    pub fn update(&mut self, f: impl FnOnce(&mut SettingsV1)) -> Result<()> { let mut s = self.load(); f(&mut s); self.save(&s) }
    pub fn save_wifi(&mut self, credentials: WifiCredentials) -> Result<()> { self.update(|s| s.wifi = Some(credentials)) }
    pub fn clear_wifi(&mut self) -> Result<()> { self.update(|s| s.wifi = None) }
    pub fn save_calibration(&mut self, calibration: TouchCalibration) -> Result<()> {
        if !calibration.valid() { return Err(anyhow!("touch calibration span is degenerate")); }
        self.update(|s| s.touch_calibration = Some(calibration))
    }
    pub fn set_brightness(&mut self, percent: u8) -> Result<()> { self.update(|s| s.brightness_percent = percent.clamp(1, 100)) }
    pub fn set_default_screen(&mut self, screen: ScreenId) -> Result<()> { self.update(|s| s.default_screen = screen) }
}

#[allow(dead_code)]
fn _expiry_types_are_available() { let _ = (Duration::ZERO, SystemTime::UNIX_EPOCH); }

#[cfg(target_os = "espidf")]
mod esp_nvs_boundary {
    //! ESP-IDF NVS integration is intentionally isolated here. Secrets are never
    //! serialized or logged by this settings boundary.
    use esp_idf_svc::nvs::EspDefaultNvs;
    #[allow(dead_code)] type NvsHandle = EspDefaultNvs;
}
