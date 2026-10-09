//! Independent quota-only adapter for the private Claude OAuth usage endpoint.
//! Reference: raycast/extensions agent-usage src/claude/{fetcher.ts,types.ts}
//! at 9a0386d3493961ab804234865d760f3c472f9145. No Raycast code is imported.
//! Only weekly model limits have a documented mapping. Extra usage is not part
//! of the snapshot contract; absent monetary values are never synthesized.

use std::{borrow::Cow, collections::BTreeMap, path::PathBuf, time::Duration};

use chrono::{DateTime, Utc};
use quota_core::{DataSource, ErrorCode, MAX_WINDOWS, Provider, ProviderState, Snapshot, Window};
use reqwest::{
    Client, Response, StatusCode,
    header::{HeaderMap, HeaderValue, RETRY_AFTER},
};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::MutexGuard;

use crate::{
    auth::claude_credentials::{CLAUDE_WRITER, Credentials},
    providers::{Collector, ProviderError},
};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const REFRESH_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const MAX_RESPONSE_BYTES: usize = 65_536;

pub struct ClaudeCollector {
    config_dir: PathBuf,
    timeout: Duration,
    client: Client,
    // Production endpoints are fixed. Owned values are used only by module tests.
    usage_url: Cow<'static, str>,
    refresh_url: Cow<'static, str>,
}

impl ClaudeCollector {
    pub fn new(config_dir: PathBuf, timeout: Duration) -> Result<Self, ProviderError> {
        let client = Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| ProviderError::new(ErrorCode::Network))?;
        Ok(Self {
            config_dir,
            timeout,
            client,
            usage_url: Cow::Borrowed(USAGE_URL),
            refresh_url: Cow::Borrowed(REFRESH_URL),
        })
    }

    async fn usage(&self, credentials: &Credentials) -> Result<Response, ProviderError> {
        // Invalid header bytes never reach reqwest errors or logs.
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {}", credentials.access_token))
                .map_err(|_| invalid())?;
        authorization.set_sensitive(true);
        self.client
            .get(self.usage_url.as_ref())
            .header(reqwest::header::AUTHORIZATION, authorization)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("anthropic-beta", OAUTH_BETA)
            .send()
            .await
            .map_err(network_error)
    }

    async fn refresh(
        &self,
        credentials: Credentials,
        guard: MutexGuard<'static, ()>,
    ) -> Result<(Credentials, MutexGuard<'static, ()>), ProviderError> {
        let refresh_token = credentials
            .refresh_token
            .as_deref()
            .ok_or_else(|| ProviderError::new(ErrorCode::Unauthenticated))?;
        let response = self
            .client
            .post(self.refresh_url.as_ref())
            .header(reqwest::header::ACCEPT, "application/json")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", CLIENT_ID),
            ])
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if status == StatusCode::BAD_REQUEST {
            return Err(ProviderError::new(ErrorCode::AuthRejected));
        }
        check_status(&response)?;
        let refreshed: RefreshResponse =
            serde_json::from_slice(&response_bytes(response).await?).map_err(|_| invalid())?;
        if refreshed.access_token.trim().is_empty()
            || refreshed
                .refresh_token
                .as_ref()
                .is_some_and(|token| token.trim().is_empty())
            || refreshed.expires_in <= 0
        {
            return Err(invalid());
        }
        HeaderValue::from_str(&refreshed.access_token).map_err(|_| invalid())?;
        let expires_at_ms = refreshed
            .expires_in
            .checked_mul(1000)
            .and_then(|duration| Utc::now().timestamp_millis().checked_add(duration))
            .ok_or_else(invalid)?;
        let config_dir = self.config_dir.clone();
        // Move the guard into the blocking write. Cancellation cannot release it
        // until fsync/rename finishes, preventing two writers from racing.
        tokio::task::spawn_blocking(move || {
            credentials
                .persist_refresh(
                    &config_dir,
                    refreshed.access_token,
                    refreshed.refresh_token,
                    expires_at_ms,
                )
                .map(|credentials| (credentials, guard))
        })
        .await
        .map_err(|_| ProviderError::new(ErrorCode::Cache))?
    }

    async fn fetch_inner(&self) -> Result<Snapshot, ProviderError> {
        let guard = CLAUDE_WRITER.lock().await;
        let config_dir = self.config_dir.clone();
        let credentials = tokio::task::spawn_blocking(move || Credentials::load(&config_dir))
            .await
            .map_err(|_| ProviderError::new(ErrorCode::Cache))??;
        let proactively_refreshed = credentials.refresh_token.is_some()
            && credentials.expires_at_ms.is_some_and(|expires| {
                expires.saturating_sub(60_000) <= Utc::now().timestamp_millis()
            });
        let (credentials, guard) = if proactively_refreshed {
            self.refresh(credentials, guard).await?
        } else {
            (credentials, guard)
        };
        let mut response = self.usage(&credentials).await?;
        let _guard = if response.status() == StatusCode::UNAUTHORIZED
            && !proactively_refreshed
            && credentials.refresh_token.is_some()
        {
            // One refresh and one usage retry maximum, including proactive refresh.
            drop(response);
            let (credentials, guard) = self.refresh(credentials, guard).await?;
            response = self.usage(&credentials).await?;
            guard
        } else {
            guard
        };
        check_status(&response)?;
        let value: Value =
            serde_json::from_slice(&response_bytes(response).await?).map_err(|_| invalid())?;
        parse_usage(value, Utc::now().timestamp())
    }
}

impl Collector for ClaudeCollector {
    async fn fetch(&mut self) -> Result<Snapshot, ProviderError> {
        // Bound the whole transaction, not each request independently.
        tokio::time::timeout(self.timeout, self.fetch_inner())
            .await
            .map_err(|_| ProviderError::new(ErrorCode::Timeout))?
    }
}

#[derive(Deserialize)]
struct RefreshResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
}

#[derive(Deserialize)]
struct OAuthWindow {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Deserialize)]
struct ScopedLimit {
    percent: Option<f64>,
    resets_at: Option<String>,
    scope: Option<LimitScope>,
    is_active: Option<bool>,
}

#[derive(Deserialize)]
struct LimitScope {
    model: Option<LimitModel>,
    surface: Option<String>,
}

#[derive(Deserialize)]
struct LimitModel {
    id: Option<String>,
    display_name: Option<String>,
}

fn invalid() -> ProviderError {
    ProviderError::new(ErrorCode::InvalidResponse)
}

fn network_error(error: reqwest::Error) -> ProviderError {
    ProviderError::new(if error.is_timeout() {
        ErrorCode::Timeout
    } else {
        ErrorCode::Network
    })
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?;
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let date = DateTime::parse_from_rfc2822(value).ok()?;
    Some(Duration::from_secs(
        date.timestamp()
            .saturating_sub(Utc::now().timestamp())
            .max(0) as u64,
    ))
}

fn check_status(response: &Response) -> Result<(), ProviderError> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let code = match status {
        StatusCode::UNAUTHORIZED => ErrorCode::AuthRejected,
        StatusCode::FORBIDDEN => ErrorCode::AccessDenied,
        StatusCode::TOO_MANY_REQUESTS => {
            return Err(match retry_after(response.headers()) {
                Some(duration) => ProviderError::with_retry_after(ErrorCode::RateLimited, duration),
                None => ProviderError::new(ErrorCode::RateLimited),
            });
        }
        StatusCode::REQUEST_TIMEOUT => ErrorCode::Timeout,
        _ if status.is_server_error() => ErrorCode::Upstream,
        _ => ErrorCode::InvalidResponse,
    };
    Err(ProviderError::new(code))
}

async fn response_bytes(mut response: Response) -> Result<Vec<u8>, ProviderError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(invalid());
    }
    let mut bytes = Vec::with_capacity(response.content_length().unwrap_or(0) as usize);
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(invalid());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn reset_timestamp(reset: Option<&str>) -> Option<i64> {
    reset
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|time| time.timestamp())
}

fn window(
    key: &str,
    label: &str,
    duration: u32,
    usage: Option<f64>,
    reset: Option<&str>,
) -> Result<Window, ProviderError> {
    Window::new(
        "claude",
        key,
        label,
        Some(duration),
        usage,
        reset_timestamp(reset),
    )
    .map_err(|_| invalid())
}

/// "sonnet" -> "Weekly Sonnet". Bounded to the snapshot's 64-byte label limit so an
/// unusually long model name cannot fail the whole fetch.
fn weekly_label(model: &str) -> String {
    let mut chars = model.chars();
    let mut label = String::from("Weekly ");
    if let Some(first) = chars.next() {
        label.extend(first.to_uppercase());
        label.push_str(chars.as_str());
    }
    while label.len() > 64 {
        label.pop();
    }
    label
}

fn parse_usage(value: Value, now: i64) -> Result<Snapshot, ProviderError> {
    let object = value.as_object().ok_or_else(invalid)?;
    let mut windows = BTreeMap::new();
    for (key, value) in object {
        let (label, duration) = match key.as_str() {
            "five_hour" => ("5-hour".to_owned(), 300),
            "seven_day" => ("Weekly".to_owned(), 10_080),
            _ if key.starts_with("seven_day_") && key.len() > "seven_day_".len() => {
                (weekly_label(&key["seven_day_".len()..]), 10_080)
            }
            _ => continue,
        };
        if value.is_null() {
            continue;
        }
        let parsed = OAuthWindow::deserialize(value).map_err(|_| invalid())?;
        windows.insert(
            key.clone(),
            window(
                key,
                &label,
                duration,
                parsed.utilization,
                parsed.resets_at.as_deref(),
            )?,
        );
        if windows.len() > MAX_WINDOWS {
            return Err(invalid());
        }
    }
    if let Some(limits) = object.get("limits").filter(|value| !value.is_null()) {
        for value in limits.as_array().ok_or_else(invalid)? {
            if value.is_null() {
                continue;
            }
            let fields = value.as_object().ok_or_else(invalid)?;
            if fields.get("kind").and_then(Value::as_str) != Some("weekly_scoped") {
                // Unknown limit kinds have no documented duration or model mapping.
                continue;
            }
            let limit = ScopedLimit::deserialize(value).map_err(|_| invalid())?;
            if limit.is_active == Some(false) {
                continue;
            }
            let Some(scope) = limit.scope else { continue };
            let Some(model) = scope.model else { continue };
            let Some(label) = model
                .display_name
                .filter(|value| !value.is_empty())
                .or(model.id.filter(|value| !value.is_empty()))
            else {
                continue;
            };
            let mut key = format!("seven_day_{}", label.to_lowercase());
            if let Some(surface) = scope.surface.filter(|value| !value.is_empty()) {
                key.push(':');
                key.push_str(&surface);
            }
            // Structured limits override equivalent flat model buckets. Separate
            // surfaces remain separate; a missing percentage remains unknown.
            windows.insert(
                key.clone(),
                window(
                    &key,
                    &weekly_label(&label),
                    10_080,
                    limit.percent,
                    limit.resets_at.as_deref(),
                )?,
            );
            if windows.len() > MAX_WINDOWS {
                return Err(invalid());
            }
        }
    }
    if windows.is_empty() {
        return Err(invalid());
    }
    let mut snapshot = Snapshot::empty(Provider::Claude, DataSource::Provider, now);
    snapshot.state = ProviderState::Ok;
    snapshot.fetched_at = Some(now);
    snapshot.windows = windows.into_values().collect();
    snapshot.validate().map_err(|_| invalid())?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        sync::Arc,
    };
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::Mutex,
        task::JoinHandle,
    };

    const USAGE: &str = include_str!("../../../../fixtures/claude-usage.json");
    const CREDENTIALS: &str =
        include_str!("../../../../fixtures/claude-synthetic-credentials.json");

    #[derive(Clone)]
    struct RecordedRequest {
        path: String,
        headers: String,
        body: String,
    }

    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
        delay: Duration,
    }

    impl Reply {
        fn json(status: u16, body: &str) -> Self {
            Self {
                status,
                headers: Vec::new(),
                body: body.into(),
                delay: Duration::ZERO,
            }
        }

        fn refresh(rotated: bool) -> Self {
            Self::json(
                200,
                if rotated {
                    r#"{"access_token":"synthetic-access-new","refresh_token":"synthetic-refresh-new","expires_in":3600}"#
                } else {
                    r#"{"access_token":"synthetic-access-new","expires_in":3600}"#
                },
            )
        }
    }

    struct FakeProvider {
        base: String,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        worker: JoinHandle<()>,
    }

    impl FakeProvider {
        async fn new(replies: Vec<Reply>) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let recorded = requests.clone();
            let worker = tokio::spawn(async move {
                for reply in replies {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut bytes = Vec::new();
                    let end = loop {
                        let mut chunk = [0; 2048];
                        let count = socket.read(&mut chunk).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&chunk[..count]);
                        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
                        {
                            break end + 4;
                        }
                    };
                    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    while bytes.len() < end + length {
                        let mut chunk = [0; 2048];
                        let count = socket.read(&mut chunk).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&chunk[..count]);
                    }
                    recorded.lock().await.push(RecordedRequest {
                        path: headers
                            .lines()
                            .next()
                            .unwrap()
                            .split_whitespace()
                            .nth(1)
                            .unwrap()
                            .into(),
                        headers: headers.to_lowercase(),
                        body: String::from_utf8(bytes[end..end + length].to_vec()).unwrap(),
                    });
                    tokio::time::sleep(reply.delay).await;
                    let mut response = format!(
                        "HTTP/1.1 {} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                        reply.status,
                        reply.body.len(),
                    );
                    for (name, value) in reply.headers {
                        response.push_str(&format!("{name}: {value}\r\n"));
                    }
                    response.push_str("\r\n");
                    response.push_str(&reply.body);
                    let _ = socket.write_all(response.as_bytes()).await;
                }
            });
            Self {
                base,
                requests,
                worker,
            }
        }

        fn collector(&self, directory: &TempDir, timeout: Duration) -> ClaudeCollector {
            let mut collector = ClaudeCollector::new(directory.path().into(), timeout).unwrap();
            collector.usage_url = Cow::Owned(format!("{}/usage", self.base));
            collector.refresh_url = Cow::Owned(format!("{}/token", self.base));
            collector
        }

        fn requests(&self) -> Vec<RecordedRequest> {
            self.requests
                .try_lock()
                .expect("request recording is idle after response")
                .clone()
        }
    }

    impl Drop for FakeProvider {
        fn drop(&mut self) {
            self.worker.abort();
        }
    }

    fn directory(contents: Option<&str>) -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        if let Some(contents) = contents {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(directory.path().join(".credentials.json"))
                .unwrap();
            file.write_all(contents.as_bytes()).unwrap();
        }
        directory
    }

    fn credential_value(directory: &TempDir) -> Value {
        serde_json::from_slice(&fs::read(directory.path().join(".credentials.json")).unwrap())
            .unwrap()
    }

    #[tokio::test]
    async fn quota_boundary_preserves_known_unknown_and_model_windows() {
        let directory = directory(Some(CREDENTIALS));
        let server = FakeProvider::new(vec![Reply::json(200, USAGE)]).await;
        let snapshot = server
            .collector(&directory, Duration::from_secs(2))
            .fetch()
            .await
            .unwrap();
        assert_eq!(snapshot.provider, Provider::Claude);
        assert_eq!(snapshot.source, DataSource::Provider);
        assert_eq!(snapshot.state, ProviderState::Ok);
        assert_eq!(snapshot.windows.len(), 5);
        let find = |key| {
            snapshot
                .windows
                .iter()
                .find(|window| window.key == key)
                .unwrap()
        };
        assert_eq!(find("five_hour").used_percent, Some(12.5));
        assert_eq!(find("five_hour").remaining_percent, Some(87.5));
        assert_eq!(find("five_hour").duration_minutes, Some(300));
        assert_eq!(
            find("five_hour").resets_at,
            Some(
                DateTime::parse_from_rfc3339("2030-01-02T03:04:05Z")
                    .unwrap()
                    .timestamp(),
            )
        );
        assert_eq!(find("seven_day").used_percent, Some(120.0));
        assert_eq!(find("seven_day").remaining_percent, Some(0.0));
        assert_eq!(
            find("seven_day").resets_at,
            Some(
                DateTime::parse_from_rfc3339("2030-01-07T03:00:00Z")
                    .unwrap()
                    .timestamp(),
            )
        );
        assert_eq!(find("seven_day_sonnet").used_percent, Some(34.5));
        assert_eq!(find("seven_day_fable").resets_at, None);
        assert_eq!(find("seven_day_future").used_percent, None);
        assert_eq!(find("seven_day_future").remaining_percent, None);
        assert!(snapshot.banked_resets.is_none());
        let payload = serde_json::to_string(&snapshot).unwrap();
        assert!(!payload.contains("synthetic-access"));
        assert!(!payload.contains("synthetic-refresh"));
        assert!(!payload.contains("extra_usage"));
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/usage");
        assert!(
            requests[0]
                .headers
                .contains("authorization: bearer synthetic-access-old")
        );
        assert!(
            requests[0]
                .headers
                .contains("anthropic-beta: oauth-2025-04-20")
        );
    }

    #[tokio::test]
    async fn absent_buckets_are_not_created_and_surface_limits_stay_separate() {
        let directory = directory(Some(CREDENTIALS));
        let server = FakeProvider::new(vec![Reply::json(200, r#"{
            "seven_day":{"utilization":null,"resets_at":null},
            "limits":[
                {"kind":"weekly_scoped","percent":15,"scope":{"model":{"id":"sonnet"},"surface":"cli"}},
                {"kind":"weekly_scoped","percent":25,"scope":{"model":{"id":"sonnet"},"surface":"web"}}
            ]}"#)]).await;
        let snapshot = server
            .collector(&directory, Duration::from_secs(2))
            .fetch()
            .await
            .unwrap();
        assert_eq!(snapshot.windows.len(), 3);
        assert!(
            !snapshot
                .windows
                .iter()
                .any(|window| window.key == "five_hour")
        );
        assert!(
            snapshot
                .windows
                .iter()
                .any(|window| window.key == "seven_day_sonnet:cli")
        );
        assert!(
            snapshot
                .windows
                .iter()
                .any(|window| window.key == "seven_day_sonnet:web")
        );
        assert!(
            snapshot
                .windows
                .iter()
                .find(|window| window.key == "seven_day")
                .unwrap()
                .remaining_percent
                .is_none()
        );
    }

    #[tokio::test]
    async fn malformed_or_missing_usage_json_is_safe() {
        for body in [
            "not-json",
            "null",
            "{}",
            r#"{"five_hour":{"utilization":"bad"}}"#,
            r#"{"five_hour":{"utilization":-1}}"#,
            r#"{"limits":{}}"#,
        ] {
            let directory = directory(Some(CREDENTIALS));
            let server = FakeProvider::new(vec![Reply::json(200, body)]).await;
            let error = server
                .collector(&directory, Duration::from_secs(2))
                .fetch()
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidResponse);
            assert!(!error.to_string().contains(body));
            assert_eq!(server.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn missing_malformed_or_unscoped_credentials_never_make_requests() {
        for (contents, expected) in [
            (None, ErrorCode::Unauthenticated),
            (Some("not-json"), ErrorCode::InvalidResponse),
            (Some("{}"), ErrorCode::InvalidResponse),
            (
                Some(r#"{"claudeAiOauth":{"scopes":["user:profile"]}}"#),
                ErrorCode::Unauthenticated,
            ),
            (
                Some(r#"{"claudeAiOauth":{"accessToken":"synthetic","scopes":[]}}"#),
                ErrorCode::AccessDenied,
            ),
            (
                Some(
                    r#"{"claudeAiOauth":{"accessToken":"synthetic","scopes":["user:profile"],"expiresAt":"bad"}}"#,
                ),
                ErrorCode::InvalidResponse,
            ),
        ] {
            let directory = directory(contents);
            let server = FakeProvider::new(vec![Reply::json(200, USAGE)]).await;
            let error = server
                .collector(&directory, Duration::from_secs(2))
                .fetch()
                .await
                .unwrap_err();
            assert_eq!(error.code, expected);
            assert!(server.requests().is_empty());
        }
    }

    #[tokio::test]
    async fn refresh_preserves_omitted_token_unknown_fields_owner_and_permissions() {
        let directory = directory(Some(CREDENTIALS));
        let path = directory.path().join(".credentials.json");
        let before = fs::metadata(&path).unwrap();
        let server = FakeProvider::new(vec![
            Reply::json(401, "secret-bearing upstream error"),
            Reply::refresh(false),
            Reply::json(200, USAGE),
            Reply::json(401, "secret-bearing upstream error"),
            Reply::refresh(true),
            Reply::json(200, USAGE),
        ])
        .await;
        let mut collector = server.collector(&directory, Duration::from_secs(2));
        let started = Utc::now().timestamp_millis();
        collector.fetch().await.unwrap();
        let first = credential_value(&directory);
        assert_eq!(
            first["claudeAiOauth"]["refreshToken"],
            "synthetic-refresh-old"
        );
        assert_eq!(
            first["claudeAiOauth"]["accessToken"],
            "synthetic-access-new"
        );
        assert_eq!(first["claudeAiOauth"]["futureOAuthField"]["kept"], true);
        assert_eq!(
            first["futureRootField"],
            serde_json::json!([1, "kept", null])
        );
        let expiry = first["claudeAiOauth"]["expiresAt"].as_i64().unwrap();
        assert!(expiry >= started + 3_600_000);
        assert!(expiry <= Utc::now().timestamp_millis() + 3_600_000);
        collector.fetch().await.unwrap();
        let rotated = credential_value(&directory);
        assert_eq!(
            rotated["claudeAiOauth"]["refreshToken"],
            "synthetic-refresh-new"
        );
        let after = fs::metadata(&path).unwrap();
        assert_eq!(after.permissions().mode() & 0o777, 0o600);
        assert_eq!(after.uid(), before.uid());
        assert_eq!(after.gid(), before.gid());
        assert_ne!(after.ino(), before.ino());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        let requests = server.requests();
        assert_eq!(requests.len(), 6);
        assert_eq!(requests[1].path, "/token");
        assert!(
            requests[1]
                .headers
                .contains("content-type: application/x-www-form-urlencoded")
        );
        assert!(requests[1].body.contains("grant_type=refresh_token"));
        assert!(requests[1].body.contains(&format!("client_id={CLIENT_ID}")));
        assert!(
            requests[1]
                .body
                .contains("refresh_token=synthetic-refresh-old")
        );
        assert!(
            requests[4]
                .body
                .contains("refresh_token=synthetic-refresh-old")
        );
        assert!(
            requests[2]
                .headers
                .contains("authorization: bearer synthetic-access-new")
        );
    }

    #[tokio::test]
    async fn unauthorized_is_refreshed_and_retried_only_once() {
        let directory = directory(Some(CREDENTIALS));
        let server = FakeProvider::new(vec![
            Reply::json(401, "secret"),
            Reply::refresh(true),
            Reply::json(401, "secret"),
            Reply::refresh(true),
            Reply::json(200, USAGE),
        ])
        .await;
        let error = server
            .collector(&directory, Duration::from_secs(2))
            .fetch()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthRejected);
        assert_eq!(server.requests().len(), 3);
        assert!(!error.to_string().contains("secret"));
    }

    #[tokio::test]
    async fn proactive_expiry_uses_milliseconds_and_never_refreshes_twice() {
        let mut credentials: Value = serde_json::from_str(CREDENTIALS).unwrap();
        credentials["claudeAiOauth"]["expiresAt"] =
            Value::from(Utc::now().timestamp_millis() + 30_000);
        let directory = directory(Some(&credentials.to_string()));
        let server = FakeProvider::new(vec![
            Reply::refresh(true),
            Reply::json(401, "secret"),
            Reply::refresh(true),
        ])
        .await;
        let error = server
            .collector(&directory, Duration::from_secs(2))
            .fetch()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthRejected);
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, "/token");
        assert_eq!(requests[1].path, "/usage");
    }

    #[tokio::test]
    async fn forbidden_rate_limited_and_upstream_failures_do_not_refresh_or_loop() {
        for (status, code) in [
            (403, ErrorCode::AccessDenied),
            (429, ErrorCode::RateLimited),
            (503, ErrorCode::Upstream),
        ] {
            let directory = directory(Some(CREDENTIALS));
            let mut reply = Reply::json(status, "secret-bearing upstream error");
            reply.headers.push(("Retry-After".into(), "123".into()));
            let server = FakeProvider::new(vec![reply, Reply::refresh(true)]).await;
            let error = server
                .collector(&directory, Duration::from_secs(2))
                .fetch()
                .await
                .unwrap_err();
            assert_eq!(error.code, code);
            assert_eq!(
                error.retry_after,
                if status == 429 {
                    Some(Duration::from_secs(123))
                } else {
                    None
                }
            );
            assert_eq!(server.requests().len(), 1);
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[tokio::test]
    async fn retry_after_http_date_is_honored() {
        let directory = directory(Some(CREDENTIALS));
        let mut reply = Reply::json(429, "secret");
        reply.headers.push((
            "Retry-After".into(),
            (Utc::now() + chrono::Duration::seconds(120))
                .format("%a, %d %b %Y %H:%M:%S GMT")
                .to_string(),
        ));
        let server = FakeProvider::new(vec![reply]).await;
        let error = server
            .collector(&directory, Duration::from_secs(2))
            .fetch()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::RateLimited);
        assert!((118..=120).contains(&error.retry_after.unwrap().as_secs()));
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn refresh_failure_is_finite_and_never_changes_credentials() {
        for reply in [
            Reply::json(400, "secret"),
            Reply::json(403, "secret"),
            Reply::json(200, "{}"),
            Reply::json(200, r#"{"access_token":"synthetic","expires_in":-1}"#),
            Reply::json(
                200,
                r#"{"access_token":"synthetic","refresh_token":"","expires_in":3600}"#,
            ),
        ] {
            let directory = directory(Some(CREDENTIALS));
            let before = fs::read(directory.path().join(".credentials.json")).unwrap();
            let server = FakeProvider::new(vec![
                Reply::json(401, "secret"),
                reply,
                Reply::json(200, USAGE),
            ])
            .await;
            let error = server
                .collector(&directory, Duration::from_secs(2))
                .fetch()
                .await
                .unwrap_err();
            assert!(matches!(
                error.code,
                ErrorCode::AuthRejected | ErrorCode::AccessDenied | ErrorCode::InvalidResponse
            ));
            assert_eq!(server.requests().len(), 2);
            assert_eq!(
                fs::read(directory.path().join(".credentials.json")).unwrap(),
                before
            );
        }
    }

    #[tokio::test]
    async fn unsafe_files_and_symlink_credentials_are_rejected() {
        let directory = directory(Some(CREDENTIALS));
        let path = directory.path().join(".credentials.json");
        let server = FakeProvider::new(vec![Reply::json(200, USAGE)]).await;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let error = server
            .collector(&directory, Duration::from_secs(2))
            .fetch()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Cache);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let other = directory.path().join("synthetic.json");
        fs::rename(&path, &other).unwrap();
        std::os::unix::fs::symlink(&other, &path).unwrap();
        let error = server
            .collector(&directory, Duration::from_secs(2))
            .fetch()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Cache);
        assert!(server.requests().is_empty());
    }

    #[tokio::test]
    async fn request_and_whole_refresh_transaction_have_finite_timeout() {
        let directory = directory(Some(CREDENTIALS));
        let mut delayed = Reply::json(200, USAGE);
        delayed.delay = Duration::from_millis(400);
        let server = FakeProvider::new(vec![delayed]).await;
        let error = server
            .collector(&directory, Duration::from_millis(100))
            .fetch()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Timeout);
        let mut usage = Reply::json(401, "secret");
        usage.delay = Duration::from_millis(80);
        let mut refresh = Reply::refresh(true);
        refresh.delay = Duration::from_millis(80);
        let server = FakeProvider::new(vec![usage, refresh, Reply::json(200, USAGE)]).await;
        let error = server
            .collector(&directory, Duration::from_millis(120))
            .fetch()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Timeout);
        assert!(server.requests().len() <= 2);
    }

    #[tokio::test]
    async fn concurrent_collectors_serialize_rotating_refresh() {
        let directory = directory(Some(CREDENTIALS));
        let mut credentials = credential_value(&directory);
        credentials["claudeAiOauth"]["expiresAt"] = Value::from(0);
        fs::write(
            directory.path().join(".credentials.json"),
            credentials.to_string(),
        )
        .unwrap();
        let server = FakeProvider::new(vec![
            Reply::refresh(true),
            Reply::json(200, USAGE),
            Reply::json(200, USAGE),
        ])
        .await;
        let mut first = server.collector(&directory, Duration::from_secs(2));
        let mut second = server.collector(&directory, Duration::from_secs(2));
        let (first, second) = tokio::join!(first.fetch(), second.fetch());
        first.unwrap();
        second.unwrap();
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/token")
                .count(),
            1
        );
        assert!(
            requests[1]
                .headers
                .contains("authorization: bearer synthetic-access-new")
        );
        assert!(
            requests[2]
                .headers
                .contains("authorization: bearer synthetic-access-new")
        );
    }

    #[tokio::test]
    async fn unauthorized_without_refresh_token_never_calls_refresh_endpoint() {
        let mut credentials: Value = serde_json::from_str(CREDENTIALS).unwrap();
        credentials["claudeAiOauth"]
            .as_object_mut()
            .unwrap()
            .remove("refreshToken");
        let directory = directory(Some(&credentials.to_string()));
        let server =
            FakeProvider::new(vec![Reply::json(401, "secret"), Reply::refresh(true)]).await;
        let error = server
            .collector(&directory, Duration::from_secs(2))
            .fetch()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthRejected);
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn redirects_and_oversized_usage_are_rejected_without_following_or_refreshing() {
        let directory = directory(Some(CREDENTIALS));
        let mut redirect = Reply::json(302, "secret");
        redirect
            .headers
            .push(("Location".into(), "/unexpected".into()));
        let server = FakeProvider::new(vec![redirect, Reply::json(200, USAGE)]).await;
        let error = server
            .collector(&directory, Duration::from_secs(2))
            .fetch()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidResponse);
        assert_eq!(server.requests().len(), 1);
        let server =
            FakeProvider::new(vec![Reply::json(200, &" ".repeat(MAX_RESPONSE_BYTES + 1))]).await;
        let error = server
            .collector(&directory, Duration::from_secs(2))
            .fetch()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidResponse);
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn oversized_window_sets_and_unknown_optional_usage_do_not_fabricate_values() {
        let directory = directory(Some(CREDENTIALS));
        let mut value = serde_json::Map::new();
        for index in 0..=MAX_WINDOWS {
            value.insert(
                format!("seven_day_model{index}"),
                serde_json::json!({"utilization": 1}),
            );
        }
        let server =
            FakeProvider::new(vec![Reply::json(200, &Value::Object(value).to_string())]).await;
        let error = server
            .collector(&directory, Duration::from_secs(2))
            .fetch()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidResponse);
        let server = FakeProvider::new(vec![Reply::json(
            200,
            r#"{
            "five_hour":{"utilization":null},
            "extra_usage":{"is_enabled":true,"monthly_limit":null,"used_credits":null},
            "limits":[null,{"kind":"future_unknown","percent":{"unknown":true}}]
        }"#,
        )])
        .await;
        let snapshot = server
            .collector(&directory, Duration::from_secs(2))
            .fetch()
            .await
            .unwrap();
        assert_eq!(snapshot.windows.len(), 1);
        assert_eq!(snapshot.windows[0].used_percent, None);
        assert_eq!(snapshot.windows[0].remaining_percent, None);
        assert_eq!(snapshot.windows[0].resets_at, None);
        assert!(snapshot.banked_resets.is_none());
    }

    #[tokio::test]
    async fn network_failure_returns_only_safe_network_enum() {
        let directory = directory(Some(CREDENTIALS));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let mut collector =
            ClaudeCollector::new(directory.path().into(), Duration::from_secs(2)).unwrap();
        collector.usage_url = Cow::Owned(format!("http://{address}/usage"));
        let error = collector.fetch().await.unwrap_err();
        assert_eq!(error.code, ErrorCode::Network);
        assert_eq!(error.to_string(), "network");
        assert_eq!(error.retry_after, None);
    }
}
