use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_SNAPSHOT_BYTES: usize = 16_384;
pub const MAX_WINDOWS: usize = 32;
pub const MAX_KNOWN_EXPIRATIONS: usize = 64;
pub const SERVER_STATUS_TOPIC: &str = "ai-quota/v1/server/status";
pub const DEVICE_CONFIG_TOPIC: &str = "ai-quota/v1/device/config";
pub const DEVICE_STATUS_TOPIC: &str = "ai-quota/v1/device/status";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
}
impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
    pub fn topic(self) -> &'static str {
        match self {
            Self::Codex => "ai-quota/v1/providers/codex",
            Self::Claude => "ai-quota/v1/providers/claude",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderState {
    Ok,
    Stale,
    ReauthRequired,
    RateLimited,
    Unavailable,
    Error,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataSource {
    #[default]
    Provider,
    Mock,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Disabled,
    Unauthenticated,
    AuthRejected,
    AccessDenied,
    RateLimited,
    Network,
    Timeout,
    InvalidResponse,
    ProcessUnavailable,
    Unsupported,
    Cache,
    Upstream,
}
impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Unauthenticated => "unauthenticated",
            Self::AuthRejected => "auth_rejected",
            Self::AccessDenied => "access_denied",
            Self::RateLimited => "rate_limited",
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::InvalidResponse => "invalid_response",
            Self::ProcessUnavailable => "process_unavailable",
            Self::Unsupported => "unsupported",
            Self::Cache => "cache",
            Self::Upstream => "upstream",
        }
    }
    pub fn state(self) -> ProviderState {
        match self {
            Self::Unauthenticated | Self::AuthRejected => ProviderState::ReauthRequired,
            Self::RateLimited => ProviderState::RateLimited,
            Self::InvalidResponse | Self::Cache => ProviderState::Error,
            _ => ProviderState::Unavailable,
        }
    }
    pub fn requires_manual_action(self) -> bool {
        matches!(
            self,
            Self::Unauthenticated
                | Self::AuthRejected
                | Self::AccessDenied
                | Self::Disabled
                | Self::Unsupported
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetailsStatus {
    Complete,
    Partial,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub limit_id: String,
    pub key: String,
    pub label: String,
    pub duration_minutes: Option<u32>,
    pub used_percent: Option<f64>,
    pub remaining_percent: Option<f64>,
    pub resets_at: Option<i64>,
}
impl Window {
    pub fn new(
        limit_id: impl Into<String>,
        key: impl Into<String>,
        label: impl Into<String>,
        duration_minutes: Option<u32>,
        used_percent: Option<f64>,
        resets_at: Option<i64>,
    ) -> Result<Self, ValidationError> {
        let result = Self {
            limit_id: limit_id.into(),
            key: key.into(),
            label: label.into(),
            duration_minutes,
            used_percent,
            remaining_percent: used_percent.map(|used| (100.0 - used).clamp(0.0, 100.0)),
            resets_at,
        };
        result.validate()?;
        Ok(result)
    }
    fn validate(&self) -> Result<(), ValidationError> {
        for text in [&self.limit_id, &self.key, &self.label] {
            if text.is_empty() || text.len() > 64 || text.chars().any(char::is_control) {
                return Err(ValidationError::Window);
            }
        }
        if self.duration_minutes == Some(0) || self.resets_at.is_some_and(|time| time < 0) {
            return Err(ValidationError::Window);
        }
        match (self.used_percent, self.remaining_percent) {
            (Some(used), Some(remaining))
                if used.is_finite()
                    && used >= 0.0
                    && remaining.is_finite()
                    && (remaining - (100.0 - used).clamp(0.0, 100.0)).abs() < 0.000001 =>
            {
                Ok(())
            }
            (None, None) => Ok(()),
            _ => Err(ValidationError::Window),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BankedResets {
    pub available_count: Option<u32>,
    pub details_status: DetailsStatus,
    pub known_expirations_at: Vec<i64>,
    pub known_nonexpiring_count: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema_version: u8,
    pub provider: Provider,
    pub state: ProviderState,
    #[serde(default)]
    pub source: DataSource,
    pub fetched_at: Option<i64>,
    pub published_at: i64,
    pub stale_after_seconds: u64,
    pub windows: Vec<Window>,
    pub banked_resets: Option<BankedResets>,
    pub error_code: Option<ErrorCode>,
}
impl Snapshot {
    pub fn empty(provider: Provider, source: DataSource, now: i64) -> Self {
        Self {
            schema_version: 1,
            provider,
            state: ProviderState::Unavailable,
            source,
            fetched_at: None,
            published_at: now,
            stale_after_seconds: 900,
            windows: Vec::new(),
            banked_resets: None,
            error_code: None,
        }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != 1
            || self.published_at < 0
            || self.fetched_at.is_some_and(|time| time < 0)
            || self.stale_after_seconds == 0
            || self.windows.len() > MAX_WINDOWS
        {
            return Err(ValidationError::Snapshot);
        }
        if self.state == ProviderState::Ok
            && (self.fetched_at.is_none() || self.error_code.is_some())
        {
            return Err(ValidationError::Snapshot);
        }
        for (index, window) in self.windows.iter().enumerate() {
            window.validate()?;
            if self.windows[..index]
                .iter()
                .any(|other| other.limit_id == window.limit_id && other.key == window.key)
            {
                return Err(ValidationError::Window);
            }
        }
        if let Some(resets) = &self.banked_resets {
            if self.provider != Provider::Codex
                || resets.known_expirations_at.len() > MAX_KNOWN_EXPIRATIONS
                || resets.known_expirations_at.iter().any(|time| *time < 0)
            {
                return Err(ValidationError::Snapshot);
            }
        }
        Ok(())
    }
    pub fn failed(&self, code: ErrorCode, published_at: i64) -> Self {
        let mut result = self.clone();
        result.state = code.state();
        result.error_code = Some(code);
        result.published_at = published_at;
        result
    }
    pub fn freshness(&self, now: Option<i64>) -> Freshness {
        match (now, self.fetched_at) {
            (None, _) => Freshness::ClockUnknown,
            (_, None) => Freshness::Unavailable,
            (Some(now), Some(fetched)) if now < fetched => Freshness::ClockAhead,
            (Some(now), Some(fetched)) if (now - fetched) as u64 > self.stale_after_seconds => {
                Freshness::Stale
            }
            _ => Freshness::Fresh,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale,
    Unavailable,
    ClockUnknown,
    ClockAhead,
}
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("invalid normalized snapshot")]
    Snapshot,
    #[error("invalid normalized quota window")]
    Window,
    #[error("snapshot exceeds protocol size limit")]
    Size,
}

pub fn decode_snapshot(topic: &str, bytes: &[u8]) -> Result<Snapshot, ValidationError> {
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(ValidationError::Size);
    }
    let snapshot: Snapshot =
        serde_json::from_slice(bytes).map_err(|_| ValidationError::Snapshot)?;
    snapshot.validate()?;
    if topic != snapshot.provider.topic() {
        return Err(ValidationError::Snapshot);
    }
    Ok(snapshot)
}
pub fn countdown(target: Option<i64>, now: Option<i64>) -> Option<u64> {
    target
        .zip(now)
        .map(|(target, now)| target.saturating_sub(now).max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        let mut value = Snapshot::empty(Provider::Codex, DataSource::Mock, 1_000);
        value.state = ProviderState::Ok;
        value.fetched_at = Some(1_000);
        value.windows.push(
            Window::new(
                "codex",
                "primary",
                "5-hour",
                Some(300),
                Some(62.0),
                Some(1_060),
            )
            .unwrap(),
        );
        value
    }
    #[test]
    fn normalization_and_expired_time_do_not_invent_allowance() {
        let value = snapshot();
        assert_eq!(value.windows[0].remaining_percent, Some(38.0));
        assert_eq!(countdown(value.windows[0].resets_at, Some(1_061)), Some(0));
        assert_eq!(value.windows[0].remaining_percent, Some(38.0));
        assert_eq!(
            Window::new("x", "p", "Unexpected", Some(731), Some(103.0), None)
                .unwrap()
                .remaining_percent,
            Some(0.0)
        );
        assert_eq!(
            Window::new("x", "p", "Unknown", None, None, None)
                .unwrap()
                .remaining_percent,
            None
        );
        assert!(Window::new("x", "p", "Invalid", None, Some(f64::NAN), None).is_err());
    }
    #[test]
    fn failure_keeps_last_success_timestamp_and_quota() {
        let failed = snapshot().failed(ErrorCode::RateLimited, 1_060);
        assert_eq!(failed.state, ProviderState::RateLimited);
        assert_eq!(failed.fetched_at, Some(1_000));
        assert_eq!(failed.published_at, 1_060);
        assert_eq!(failed.windows[0].remaining_percent, Some(38.0));
        assert_eq!(failed.freshness(Some(1_901)), Freshness::Stale);
        assert_eq!(failed.freshness(Some(999)), Freshness::ClockAhead);
        assert_eq!(failed.freshness(None), Freshness::ClockUnknown);
        assert_eq!(countdown(None, Some(1_000)), None);
    }
    #[test]
    fn topic_and_size_boundaries_reject_untrusted_payloads() {
        let value = serde_json::to_vec(&snapshot()).unwrap();
        assert!(decode_snapshot(Provider::Claude.topic(), &value).is_err());
        assert!(
            decode_snapshot(Provider::Codex.topic(), &vec![b' '; MAX_SNAPSHOT_BYTES + 1]).is_err()
        );
        let mut wrong = snapshot();
        wrong.schema_version = 2;
        assert!(
            decode_snapshot(
                Provider::Codex.topic(),
                &serde_json::to_vec(&wrong).unwrap()
            )
            .is_err()
        );
        wrong = snapshot();
        wrong.windows[0].remaining_percent = Some(100.0);
        assert!(wrong.validate().is_err());
    }
}
