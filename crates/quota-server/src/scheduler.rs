use crate::{
    cache::Cache,
    mqtt::Publisher,
    providers::{Collector, ProviderError},
    settings::ServerSettings,
};
use quota_core::{DataSource, ErrorCode, MAX_SNAPSHOT_BYTES, Provider, ProviderState, Snapshot};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Notify, watch};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// State shared between one provider's scheduler and the HTTP API.
/// Never held across an await.
#[derive(Default)]
pub struct Control {
    state: Mutex<Status>,
    wake: Notify,
}
#[derive(Default)]
struct Status {
    snapshot: Option<Snapshot>,
    next_at_ms: Option<i64>,
    /// No fetch may start before this instant: Retry-After or exponential backoff after a failure.
    floor_ms: i64,
    requested: bool,
}
impl Control {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }
    fn state(&self) -> std::sync::MutexGuard<'_, Status> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// What MQTT carries for this provider: last published success, failure or unavailable snapshot.
    pub fn snapshot(&self) -> Option<Snapshot> {
        self.state().snapshot.clone()
    }
    /// Unix seconds of the next automatic fetch; `None` while idle or waiting for a manual refresh.
    pub fn next_refresh_at(&self) -> Option<i64> {
        self.state()
            .next_at_ms
            .map(|ms| (ms + 999).div_euclid(1000))
    }
    /// Remaining Retry-After/backoff in whole seconds, rounded up, while one is active.
    pub fn retry_after_seconds(&self) -> Option<u64> {
        let remaining = self.state().floor_ms - now_ms();
        (remaining > 0).then(|| ((remaining + 999) / 1000) as u64)
    }
    /// Asks the scheduler to fetch now. Refused while Retry-After/backoff is active.
    pub fn request_refresh(&self) -> Result<(), u64> {
        if let Some(seconds) = self.retry_after_seconds() {
            return Err(seconds);
        }
        self.state().requested = true;
        self.wake.notify_one();
        Ok(())
    }
    fn set_snapshot(&self, snapshot: &Snapshot) {
        self.state().snapshot = Some(snapshot.clone());
    }
    fn plan(&self, next_at_ms: Option<i64>, floor_ms: i64) {
        let mut state = self.state();
        state.next_at_ms = next_at_ms;
        state.floor_ms = floor_ms;
    }
    fn clear_request(&self) {
        self.state().requested = false;
    }
    fn take_request(&self) -> bool {
        std::mem::take(&mut self.state().requested)
    }
}

fn encode(snapshot: &Snapshot) -> Result<Vec<u8>, ProviderError> {
    snapshot
        .validate()
        .map_err(|_| ProviderError::new(ErrorCode::InvalidResponse))?;
    let bytes =
        serde_json::to_vec(snapshot).map_err(|_| ProviderError::new(ErrorCode::InvalidResponse))?;
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(ProviderError::new(ErrorCode::InvalidResponse));
    }
    Ok(bytes)
}

enum Wait {
    Go,
    Disabled,
    Shutdown,
}

/// Everything one provider's scheduler needs. `run` owns the whole lifecycle:
/// disabled -> enabled -> disabled again, driven by the settings channel.
pub struct Scheduler {
    pub provider: Provider,
    pub source: DataSource,
    pub settings: watch::Receiver<ServerSettings>,
    pub publisher: Publisher,
    pub cache: Cache,
    pub control: Arc<Control>,
    pub shutdown: watch::Receiver<bool>,
}

impl Scheduler {
    fn current(&mut self) -> ServerSettings {
        self.settings.borrow_and_update().clone()
    }
    fn enabled(&mut self) -> bool {
        self.settings
            .borrow_and_update()
            .provider(self.provider)
            .enabled
    }

    async fn publish(&self, snapshot: &Snapshot, bytes: Vec<u8>, save: bool) {
        if save && self.cache.save(snapshot, &bytes).await.is_err() {
            tracing::warn!(
                provider = self.provider.as_str(),
                code = "cache",
                "Snapshot persistence failed"
            );
        }
        self.control.set_snapshot(snapshot);
        if self.publisher.publish(self.provider, bytes).await.is_err() {
            tracing::warn!(
                provider = self.provider.as_str(),
                "MQTT publication queued for reconnect"
            );
        }
    }

    async fn publish_unavailable(&self, code: ErrorCode, stale_after_seconds: u64) {
        let mut snapshot =
            Snapshot::empty(self.provider, self.source, chrono::Utc::now().timestamp());
        snapshot.error_code = Some(code);
        snapshot.state = code.state();
        snapshot.stale_after_seconds = stale_after_seconds;
        if let Ok(bytes) = encode(&snapshot) {
            self.publish(&snapshot, bytes, false).await;
        }
    }

    /// Runs until shutdown. `factory` builds a collector for a request timeout; it is called
    /// again whenever the timeout setting changes or the previous build failed.
    pub async fn run<C, F>(mut self, mut factory: F)
    where
        C: Collector,
        F: FnMut(Duration) -> Result<C, ProviderError>,
    {
        loop {
            if !self.enabled() {
                self.control.plan(None, 0);
                let stale = self.current().stale_after_seconds;
                self.publish_unavailable(ErrorCode::Disabled, stale).await;
                if !self.wait_enabled().await {
                    return;
                }
            }
            if !self.run_enabled(&mut factory).await {
                return;
            }
        }
    }

    /// `false` on shutdown.
    async fn wait_enabled(&mut self) -> bool {
        loop {
            tokio::select! {
                _ = self.shutdown.changed() => return false,
                changed = self.settings.changed() => if changed.is_err() { return false },
            }
            if self.enabled() {
                return true;
            }
        }
    }

    /// `true`: provider was disabled, caller loops. `false`: shutdown.
    async fn run_enabled<C, F>(&mut self, factory: &mut F) -> bool
    where
        C: Collector,
        F: FnMut(Duration) -> Result<C, ProviderError>,
    {
        let provider = self.provider;
        let stale_after = self.current().stale_after_seconds;
        let now = chrono::Utc::now().timestamp();
        let mut last = match self.cache.load(provider, self.source).await {
            Ok(Some(mut snapshot)) => {
                snapshot.state = ProviderState::Stale;
                snapshot.published_at = now;
                snapshot.stale_after_seconds = stale_after;
                snapshot
            }
            Ok(None) => Snapshot::empty(provider, self.source, now),
            Err(_) => {
                tracing::warn!(
                    provider = provider.as_str(),
                    code = "cache",
                    "Snapshot cache unavailable"
                );
                Snapshot::empty(provider, self.source, now)
            }
        };
        if let Ok(bytes) = encode(&last) {
            self.publish(&last, bytes, false).await;
        }
        let mut collector: Option<C> = None;
        let mut built_for = Duration::ZERO;
        let mut attempts = 0u32;
        let mut retry_after = None;
        let mut manual_only = false;
        let mut anchor_ms = now_ms();
        let mut floor_ms = 0i64;
        let mut first = true;
        loop {
            if !first {
                match self
                    .wait(anchor_ms, floor_ms, attempts, retry_after, manual_only)
                    .await
                {
                    Wait::Go => {}
                    Wait::Disabled => return true,
                    Wait::Shutdown => return false,
                }
            }
            first = false;
            self.control.plan(None, floor_ms);
            self.control.clear_request();
            let settings = self.current();
            let timeout = Duration::from_secs(settings.request_timeout_seconds);
            let mut build_error = None;
            if collector.is_none() || built_for != timeout {
                match factory(timeout) {
                    Ok(built) => {
                        collector = Some(built);
                        built_for = timeout;
                    }
                    Err(error) => {
                        collector = None;
                        build_error = Some(error);
                    }
                }
            }
            let fetched = match (collector.as_mut(), build_error) {
                (_, Some(error)) => Err(error),
                (Some(collector), None) => tokio::select! {
                    _ = self.shutdown.changed() => return false,
                    result = collector.fetch() => result,
                },
                (None, None) => Err(ProviderError::new(ErrorCode::ProcessUnavailable)),
            };
            if !self.enabled() {
                return true;
            }
            // A request that arrived mid-fetch is answered by this fetch.
            self.control.clear_request();
            let settings = self.current();
            let now = chrono::Utc::now().timestamp();
            let fetched = fetched.and_then(|mut snapshot| {
                snapshot.published_at = now;
                snapshot.stale_after_seconds = settings.stale_after_seconds;
                if snapshot.provider != provider
                    || snapshot.source != self.source
                    || snapshot.state != ProviderState::Ok
                    || snapshot.fetched_at.is_none()
                {
                    return Err(ProviderError::new(ErrorCode::InvalidResponse));
                }
                let bytes = encode(&snapshot)?;
                Ok((snapshot, bytes))
            });
            anchor_ms = now_ms();
            match fetched {
                Ok((snapshot, bytes)) => {
                    self.publish(&snapshot, bytes, true).await;
                    last = snapshot;
                    attempts = 0;
                    retry_after = None;
                    manual_only = false;
                    floor_ms = 0;
                }
                Err(error) => {
                    let failed = last.failed(error.code, now);
                    if let Ok(bytes) = encode(&failed) {
                        self.publish(&failed, bytes, false).await;
                    }
                    tracing::warn!(
                        provider = provider.as_str(),
                        code = error.code.as_str(),
                        "Provider fetch failed"
                    );
                    if error.code.requires_manual_action() {
                        // Retrying cannot help until the operator acts; wait for a manual refresh.
                        manual_only = true;
                        attempts = 0;
                        retry_after = None;
                        floor_ms = 0;
                    } else {
                        manual_only = false;
                        attempts = attempts.saturating_add(1);
                        retry_after = error.retry_after;
                        let delay = retry_delay(&settings, provider, attempts, retry_after);
                        floor_ms = anchor_ms + delay.as_millis() as i64;
                    }
                }
            }
        }
    }

    /// Sleeps until the next fetch is due, recomputing from live settings whenever they change.
    /// A shorter interval applies promptly but never before the Retry-After/backoff floor.
    async fn wait(
        &mut self,
        anchor_ms: i64,
        floor_ms: i64,
        attempts: u32,
        retry_after: Option<Duration>,
        manual_only: bool,
    ) -> Wait {
        loop {
            let settings = self.current();
            if !settings.provider(self.provider).enabled {
                return Wait::Disabled;
            }
            let target = (!manual_only).then(|| {
                let base = if attempts == 0 {
                    interval(&settings, self.provider)
                } else {
                    backoff(&settings, self.provider, attempts, retry_after)
                };
                (anchor_ms + base.as_millis() as i64).max(floor_ms)
            });
            self.control.plan(target, floor_ms);
            let sleep = match target {
                Some(target) if target <= now_ms() => return Wait::Go,
                Some(target) => Some(Duration::from_millis((target - now_ms()) as u64)),
                None => None,
            };
            tokio::select! {
                _ = self.shutdown.changed() => return Wait::Shutdown,
                changed = self.settings.changed() => if changed.is_err() { return Wait::Shutdown },
                _ = self.control.wake.notified() => {
                    if self.control.take_request() && now_ms() >= floor_ms {
                        return Wait::Go;
                    }
                }
                _ = async {
                    match sleep {
                        Some(duration) => tokio::time::sleep(duration).await,
                        None => std::future::pending().await,
                    }
                } => return Wait::Go,
            }
        }
    }
}

fn interval(settings: &ServerSettings, provider: Provider) -> Duration {
    Duration::from_secs(settings.provider(provider).refresh_interval_seconds)
}

/// Exponential backoff capped by `max_backoff_seconds`; Retry-After is never shortened by the cap.
pub fn backoff(
    settings: &ServerSettings,
    provider: Provider,
    attempts: u32,
    retry_after: Option<Duration>,
) -> Duration {
    let exponential = interval(settings, provider)
        .saturating_mul(1u32 << attempts.min(16))
        .min(Duration::from_secs(settings.max_backoff_seconds));
    retry_after.map_or(exponential, |minimum| exponential.max(minimum))
}

pub fn retry_delay(
    settings: &ServerSettings,
    provider: Provider,
    attempts: u32,
    retry_after: Option<Duration>,
) -> Duration {
    backoff(settings, provider, attempts, retry_after)
        + Duration::from_millis(rand::random_range(0..=250))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{ProviderMap, ProviderSettings};

    fn settings() -> ServerSettings {
        let provider = |seconds| ProviderSettings {
            enabled: true,
            refresh_interval_seconds: seconds,
        };
        ServerSettings {
            providers: ProviderMap {
                claude: provider(600),
                codex: provider(300),
            },
            stale_after_seconds: 900,
            max_backoff_seconds: 1800,
            request_timeout_seconds: 12,
        }
    }
    #[test]
    fn retry_after_is_never_shortened_by_backoff_cap() {
        let retry_after = Duration::from_secs(3600);
        let delay = retry_delay(&settings(), Provider::Codex, 20, Some(retry_after));
        assert!(delay >= retry_after && delay < retry_after + Duration::from_secs(1));
    }
    #[test]
    fn backoff_doubles_per_failure_and_stops_at_the_cap() {
        let settings = settings();
        let seconds = |attempts| backoff(&settings, Provider::Codex, attempts, None).as_secs();
        assert_eq!([seconds(1), seconds(2), seconds(3)], [600, 1200, 1800]);
        assert_eq!(seconds(30), 1800);
        // Per-provider interval: Claude starts from its own 600 s.
        assert_eq!(
            backoff(&settings, Provider::Claude, 1, None).as_secs(),
            1200
        );
    }
    #[test]
    fn refresh_is_refused_while_floor_active() {
        let control = Control::new();
        assert_eq!(control.request_refresh(), Ok(()));
        control.plan(None, now_ms() + 90_500);
        assert_eq!(control.request_refresh(), Err(91));
        control.plan(None, 0);
        assert_eq!(control.retry_after_seconds(), None);
    }
}
