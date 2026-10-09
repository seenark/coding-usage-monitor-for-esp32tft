use crate::providers::{Collector, ProviderError};
use quota_core::{
    BankedResets, DataSource, DetailsStatus, ErrorCode, Provider, ProviderState, Snapshot, Window,
};

pub struct MockCollector {
    provider: Provider,
}
impl MockCollector {
    pub fn new(provider: Provider) -> Self {
        Self { provider }
    }
}
impl Collector for MockCollector {
    async fn fetch(&mut self) -> Result<Snapshot, ProviderError> {
        let now = chrono::Utc::now().timestamp();
        let mut snapshot = Snapshot::empty(self.provider, DataSource::Mock, now);
        snapshot.state = ProviderState::Ok;
        snapshot.fetched_at = Some(now);
        let used = match self.provider {
            Provider::Codex => [62.0, 76.0],
            Provider::Claude => [28.0, 51.0],
        };
        snapshot.windows = vec![
            Window::new(
                self.provider.as_str(),
                "primary",
                "5-hour",
                Some(300),
                Some(used[0]),
                Some(now + 7_200),
            ),
            Window::new(
                self.provider.as_str(),
                "secondary",
                "Weekly",
                Some(10_080),
                Some(used[1]),
                Some(now + 172_800),
            ),
        ]
        .into_iter()
        .collect::<Result<_, _>>()
        .map_err(|_| ProviderError::new(ErrorCode::InvalidResponse))?;
        if self.provider == Provider::Codex {
            snapshot.banked_resets = Some(BankedResets {
                available_count: Some(2),
                details_status: DetailsStatus::Partial,
                known_expirations_at: vec![now + 604_800],
                known_nonexpiring_count: 0,
            });
        }
        Ok(snapshot)
    }
}
