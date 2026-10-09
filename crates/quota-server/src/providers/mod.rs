use quota_core::{ErrorCode, Snapshot};
use std::{fmt, future::Future, time::Duration};

pub mod claude;
pub mod codex;

#[derive(Debug, Clone, Copy)]
pub struct ProviderError {
    pub code: ErrorCode,
    pub retry_after: Option<Duration>,
}

impl ProviderError {
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            retry_after: None,
        }
    }
    pub fn with_retry_after(code: ErrorCode, retry_after: Duration) -> Self {
        Self {
            code,
            retry_after: Some(retry_after),
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code.as_str())
    }
}
impl std::error::Error for ProviderError {}

pub trait Collector: Send {
    fn fetch(&mut self) -> impl Future<Output = Result<Snapshot, ProviderError>> + Send;
}
