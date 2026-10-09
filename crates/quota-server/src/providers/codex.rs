//! Read-only Codex app-server adapter. Protocol reference: openai/codex revision
//! e974aad3b1a8f144273e882c614aefe69eaef615 (2026-10-08),
//! codex-rs/app-server-protocol/src/protocol/v2/account.rs and schema/json/v1/InitializeParams.json.
//! Transport: https://learn.chatgpt.com/docs/app-server (JSONL without a jsonrpc header).
//! Only initialize, initialized, account/read, and account/rateLimits/read are sent.
//! Authentication and refresh belong to the isolated, file-authenticated Codex child.
//! The pinned CLI's internal remote-control opt-out disables persisted remote enrollment.
//! No socket transport, shared daemon command, or external-auth callback is used.
//! Nullable credit expirations stay unknown under this monitor's conservative contract;
//! this protocol revision has no separate explicit non-expiring flag, so that count stays zero.

use super::{Collector, ProviderError};
use quota_core::{
    BankedResets, DataSource, DetailsStatus, ErrorCode, MAX_KNOWN_EXPIRATIONS, MAX_WINDOWS,
    Provider, ProviderState, Snapshot, Window,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::{Instant, timeout};

const MAX_FRAME_BYTES: usize = 128 * 1024;
const MAX_SKIPPED_MESSAGES: usize = 128;
const MAX_RESTART_BACKOFF_SECONDS: u64 = 60;

pub struct CodexCollector {
    program: PathBuf,
    home: PathBuf,
    timeout: Duration,
    session: Option<Session>,
    next_id: u64,
    failures: u32,
    retry_at: Option<Instant>,
    last_error: ErrorCode,
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    frame: Vec<u8>,
    initialized: bool,
}

impl CodexCollector {
    pub fn new(program: PathBuf, home: PathBuf, timeout: Duration) -> Self {
        Self {
            program,
            home,
            timeout,
            session: None,
            next_id: 1,
            failures: 0,
            retry_at: None,
            last_error: ErrorCode::ProcessUnavailable,
        }
    }

    fn spawn(&self) -> Result<Session, ProviderError> {
        let sqlite_home = serde_json::to_string(&self.home)
            .map_err(|_| ProviderError::new(ErrorCode::ProcessUnavailable))?;
        let mut child = Command::new(&self.program)
            .args([
                "app-server",
                "--listen",
                "stdio://",
                "-c",
                "cli_auth_credentials_store=\"file\"",
                "-c",
                "forced_login_method=\"chatgpt\"",
            ])
            .args(["-c", &format!("sqlite_home={sqlite_home}")])
            .env("CODEX_HOME", &self.home)
            .env("CODEX_SQLITE_HOME", &self.home)
            .env("CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED", "1")
            .env_remove("OPENAI_API_KEY")
            .env_remove("CODEX_API_KEY")
            .current_dir(&self.home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Upstream errors and stderr may contain account or credential material.
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| ProviderError::new(ErrorCode::ProcessUnavailable))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ProviderError::new(ErrorCode::ProcessUnavailable))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProviderError::new(ErrorCode::ProcessUnavailable))?;
        Ok(Session {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            frame: Vec::with_capacity(4096),
            initialized: false,
        })
    }

    async fn poll(session: &mut Session, next_id: &mut u64) -> Result<Snapshot, ProviderError> {
        if !session.initialized {
            let initialization = session
                .request(
                    next_id,
                    "initialize",
                    json!({
                        "clientInfo": {
                            "name": "ai_quota_monitor",
                            "title": "AI Quota Monitor",
                            "version": env!("CARGO_PKG_VERSION")
                        },
                        "capabilities": {"experimentalApi": false}
                    }),
                )
                .await?;
            if initialization
                .get("userAgent")
                .and_then(Value::as_str)
                .is_none()
            {
                return Err(invalid_response());
            }
            session
                .send(&json!({"method": "initialized", "params": {}}))
                .await?;
            session.initialized = true;
        }
        // Ask Codex to perform its managed refresh. No token ever crosses this interface.
        let account = session
            .request(next_id, "account/read", json!({"refreshToken": true}))
            .await?;
        match account.get("account") {
            Some(Value::Null) => return Err(ProviderError::new(ErrorCode::Unauthenticated)),
            Some(account) if account.get("type").and_then(Value::as_str) == Some("chatgpt") => {}
            Some(account) if account.is_object() => {
                return Err(ProviderError::new(ErrorCode::Unsupported));
            }
            _ => return Err(invalid_response()),
        }
        let response = session
            .request(
                next_id,
                "account/rateLimits/read",
                json!({"supportsLunaReserve": false, "excludeResetCreditDetails": false}),
            )
            .await?;
        normalize(response)
    }

    fn fail(&mut self, code: ErrorCode) -> ProviderError {
        self.failures = self.failures.saturating_add(1);
        let seconds =
            (1_u64 << self.failures.saturating_sub(1).min(6)).min(MAX_RESTART_BACKOFF_SECONDS);
        let delay = Duration::from_secs(seconds);
        self.last_error = code;
        self.retry_at = Some(Instant::now() + delay);
        ProviderError::with_retry_after(code, delay)
    }
}

impl Collector for CodexCollector {
    async fn fetch(&mut self) -> Result<Snapshot, ProviderError> {
        if let Some(retry_at) = self.retry_at {
            let remaining = retry_at.saturating_duration_since(Instant::now());
            if !remaining.is_zero() {
                return Err(ProviderError::with_retry_after(self.last_error, remaining));
            }
        }
        // Keep the child local while awaiting: cancelling this future kills it, rather than
        // retaining an interrupted request that could poison the next poll.
        let mut session = match self.session.take() {
            Some(session) => session,
            None => match self.spawn() {
                Ok(session) => session,
                Err(error) => return Err(self.fail(error.code)),
            },
        };
        let result = timeout(self.timeout, Self::poll(&mut session, &mut self.next_id)).await;
        match result {
            Ok(Ok(snapshot)) => {
                self.failures = 0;
                self.retry_at = None;
                self.session = Some(session);
                Ok(snapshot)
            }
            result => {
                let code = match result {
                    Ok(Err(error)) => error.code,
                    Err(_) => ErrorCode::Timeout,
                    Ok(Ok(_)) => unreachable!(),
                };
                // Kill before waiting; a broken child must not survive a timeout or framing error.
                let _ = session.child.start_kill();
                let _ = timeout(Duration::from_secs(1), session.child.wait()).await;
                Err(self.fail(code))
            }
        }
    }
}

impl Session {
    async fn send(&mut self, message: &Value) -> Result<(), ProviderError> {
        let mut bytes = serde_json::to_vec(message).map_err(|_| invalid_response())?;
        bytes.push(b'\n');
        self.stdin
            .write_all(&bytes)
            .await
            .map_err(|_| ProviderError::new(ErrorCode::ProcessUnavailable))?;
        self.stdin
            .flush()
            .await
            .map_err(|_| ProviderError::new(ErrorCode::ProcessUnavailable))
    }

    async fn request(
        &mut self,
        next_id: &mut u64,
        method: &str,
        params: Value,
    ) -> Result<Value, ProviderError> {
        let id = *next_id;
        *next_id = id.checked_add(1).ok_or_else(invalid_response)?;
        self.send(&json!({"id": id, "method": method, "params": params}))
            .await?;
        for _ in 0..=MAX_SKIPPED_MESSAGES {
            let mut response = self.read_frame().await?;
            if !response.is_object() {
                return Err(invalid_response());
            }
            if response.get("method").is_some() {
                // Never service a server request (approvals, refresh tokens, or inference).
                // File authentication is child-managed, so external auth requests are unsupported.
                if response.get("id").is_some() {
                    return Err(ProviderError::new(ErrorCode::Unsupported));
                }
                continue;
            }
            if response.get("id").and_then(Value::as_u64) != Some(id) {
                // Async notifications and late/stray responses cannot satisfy this request.
                continue;
            }
            let has_result = response.get("result").is_some();
            let has_error = response.get("error").is_some();
            if has_result == has_error {
                return Err(invalid_response());
            }
            if has_error {
                return Err(rpc_error(method, &response["error"]));
            }
            return Ok(response["result"].take());
        }
        Err(invalid_response())
    }

    async fn read_frame(&mut self) -> Result<Value, ProviderError> {
        self.frame.clear();
        loop {
            let available = self
                .stdout
                .fill_buf()
                .await
                .map_err(|_| ProviderError::new(ErrorCode::ProcessUnavailable))?;
            if available.is_empty() {
                // A final unterminated JSON object is not a complete JSONL frame.
                return Err(ProviderError::new(ErrorCode::ProcessUnavailable));
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(available.len(), |index| index + 1);
            if self.frame.len() + count > MAX_FRAME_BYTES {
                return Err(invalid_response());
            }
            self.frame.extend_from_slice(&available[..count]);
            self.stdout.consume(count);
            if newline.is_some() {
                return serde_json::from_slice(&self.frame).map_err(|_| invalid_response());
            }
        }
    }
}

fn invalid_response() -> ProviderError {
    ProviderError::new(ErrorCode::InvalidResponse)
}

fn rpc_error(method: &str, error: &Value) -> ProviderError {
    // Never classify by, retain, or expose free-form upstream message text.
    let code = match error.get("code").and_then(Value::as_i64) {
        Some(-32601) => ErrorCode::Unsupported,
        Some(-32600) if method == "account/rateLimits/read" => ErrorCode::Unauthenticated,
        Some(-32600 | -32602 | -32700) => ErrorCode::InvalidResponse,
        Some(-32001) => ErrorCode::RateLimited,
        _ => ErrorCode::Upstream,
    };
    ProviderError::new(code)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitsResponse {
    rate_limits: Option<Bucket>,
    rate_limits_by_limit_id: Option<BTreeMap<String, Bucket>>,
    rate_limit_reset_credits: Option<ResetCredits>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bucket {
    limit_id: Option<String>,
    primary: Option<UpstreamWindow>,
    secondary: Option<UpstreamWindow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpstreamWindow {
    used_percent: Option<f64>,
    window_duration_mins: Option<u32>,
    resets_at: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResetCredits {
    available_count: Option<u32>,
    credits: Option<Vec<ResetCredit>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResetCredit {
    id: Option<String>,
    reset_type: Option<String>,
    status: Option<String>,
    expires_at: Option<i64>,
}

fn normalize(response: Value) -> Result<Snapshot, ProviderError> {
    let response: RateLimitsResponse =
        serde_json::from_value(response).map_err(|_| invalid_response())?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .ok_or_else(invalid_response)?;
    let mut snapshot = Snapshot::empty(Provider::Codex, DataSource::Provider, now);
    snapshot.state = ProviderState::Ok;
    snapshot.fetched_at = Some(now);
    // The multi-bucket map is authoritative, including an explicitly empty map.
    match response.rate_limits_by_limit_id {
        Some(buckets) => {
            for (limit_id, bucket) in buckets {
                add_bucket(&mut snapshot, limit_id, bucket)?;
            }
        }
        None => {
            let bucket = response.rate_limits.ok_or_else(invalid_response)?;
            let limit_id = bucket
                .limit_id
                .clone()
                .unwrap_or_else(|| "codex".to_owned());
            add_bucket(&mut snapshot, limit_id, bucket)?;
        }
    }
    snapshot.banked_resets = response.rate_limit_reset_credits.map(normalize_credits);
    snapshot.validate().map_err(|_| invalid_response())?;
    Ok(snapshot)
}

fn add_bucket(
    snapshot: &mut Snapshot,
    limit_id: String,
    bucket: Bucket,
) -> Result<(), ProviderError> {
    if limit_id.is_empty() || limit_id.len() > 64 || limit_id.chars().any(char::is_control) {
        return Err(invalid_response());
    }
    if bucket.limit_id.as_ref().is_some_and(|id| id != &limit_id) {
        return Err(invalid_response());
    }
    for (key, fallback, window) in [
        ("primary", "Primary", bucket.primary),
        ("secondary", "Secondary", bucket.secondary),
    ] {
        let Some(window) = window else { continue };
        if snapshot.windows.len() >= MAX_WINDOWS {
            return Err(invalid_response());
        }
        // Name the window by its actual duration; the slot name says nothing to a reader.
        let label = match window.window_duration_mins {
            Some(300) => "5-hour".to_owned(),
            Some(10_080) => "Weekly".to_owned(),
            Some(minutes) if minutes % 1_440 == 0 => format!("{}-day", minutes / 1_440),
            Some(minutes) if minutes % 60 == 0 => format!("{}-hour", minutes / 60),
            _ => fallback.to_owned(),
        };
        snapshot.windows.push(
            Window::new(
                limit_id.clone(),
                key.to_owned(),
                label,
                window.window_duration_mins,
                window.used_percent,
                window.resets_at,
            )
            .map_err(|_| invalid_response())?,
        );
    }
    Ok(())
}

fn normalize_credits(summary: ResetCredits) -> BankedResets {
    let mut bank = BankedResets {
        available_count: summary.available_count,
        details_status: DetailsStatus::Unavailable,
        known_expirations_at: Vec::new(),
        known_nonexpiring_count: 0,
    };
    let Some(credits) = summary.credits else {
        return bank;
    };
    let mut complete = summary
        .available_count
        .is_some_and(|count| usize::try_from(count).ok() == Some(credits.len()));
    let mut seen = HashSet::new();
    for credit in credits {
        // Status/type uncertainty must not turn a redeemed or unrelated credit into available quota.
        if credit.status.as_deref() != Some("available")
            || credit.reset_type.as_deref() != Some("codexRateLimits")
        {
            complete = false;
            continue;
        }
        if !credit
            .id
            .filter(|id| !id.is_empty())
            .is_some_and(|id| seen.insert(id))
        {
            complete = false;
            continue;
        }
        match credit.expires_at {
            Some(expiry)
                if expiry >= 0 && bank.known_expirations_at.len() < MAX_KNOWN_EXPIRATIONS =>
            {
                bank.known_expirations_at.push(expiry);
            }
            _ => complete = false,
        }
    }
    bank.known_expirations_at.sort_unstable();
    // Count is authoritative. Detail coverage is complete only when every row was understood,
    // the full count was supplied, and no expiration was dropped by the payload cap.
    bank.details_status = if complete {
        DetailsStatus::Complete
    } else {
        DetailsStatus::Partial
    };
    bank
}
