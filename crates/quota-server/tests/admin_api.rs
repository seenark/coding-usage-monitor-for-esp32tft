//! In-process Admin API tests: real axum server on loopback, real schedulers with a scripted
//! collector, real settings files. The MQTT runtime points at a closed port, so publishes queue.
use quota_core::{DataSource, Provider};
use quota_server::{
    cache::Cache,
    config::{DeviceSetupConfig, MqttConfig},
    http::{self, AppState},
    mock::MockCollector,
    mqtt::Runtime,
    providers::{Collector, ProviderError},
    scheduler::{Control, Scheduler},
    settings::{DeviceSettings, ProviderMap, ProviderSettings, ServerSettings, Store},
};
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{net::TcpListener, sync::watch};

const SERVER_PASSWORD: &str = "server-secret-5b1f0c77";
const DEVICE_PASSWORD: &str = "483920175566";

/// Succeeds for the first `ok_calls` fetches, then fails with `network`.
struct Scripted {
    inner: MockCollector,
    calls: Arc<AtomicU32>,
    ok_calls: u32,
}
impl Collector for Scripted {
    async fn fetch(&mut self) -> Result<quota_core::Snapshot, ProviderError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) >= self.ok_calls {
            return Err(ProviderError::new(quota_core::ErrorCode::Network));
        }
        self.inner.fetch().await
    }
}

struct Harness {
    base: String,
    client: Client,
    directory: tempfile::TempDir,
    codex_calls: Arc<AtomicU32>,
    _shutdown: watch::Sender<bool>,
}

fn defaults() -> ServerSettings {
    ServerSettings {
        providers: ProviderMap {
            claude: ProviderSettings {
                enabled: false,
                refresh_interval_seconds: 600,
            },
            codex: ProviderSettings {
                enabled: true,
                refresh_interval_seconds: 300,
            },
        },
        stale_after_seconds: 900,
        max_backoff_seconds: 1800,
        request_timeout_seconds: 12,
    }
}

/// `codex_ok_calls`: successful Codex fetches before the scripted collector starts failing.
async fn start(codex_ok_calls: u32, with_dist: bool, device_setup: bool) -> Harness {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let cache_dir = root.join("cache");
    let dist = root.join("web");
    if with_dist {
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><title>admin</title>",
        )
        .unwrap();
        std::fs::write(dist.join("assets/app-abc123.js"), "console.log(1)").unwrap();
    }
    std::fs::write(
        root.join("mqtt-server-password"),
        format!("{SERVER_PASSWORD}\n"),
    )
    .unwrap();
    std::fs::write(
        root.join("mqtt-device-password"),
        format!("{DEVICE_PASSWORD}\n"),
    )
    .unwrap();
    let closed_port = {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        probe.local_addr().unwrap().port()
    };
    let mqtt = Runtime::new(MqttConfig {
        host: "127.0.0.1".into(),
        port: closed_port,
        username: "quota-server".into(),
        password_file: root.join("mqtt-server-password"),
        client_id: "quota-admin-api-test".into(),
        tls: false,
        ca_file: None,
    })
    .await
    .unwrap();
    let cache = Cache::new(&cache_dir).await.unwrap();
    let store = Store::new(&cache_dir);
    let (settings_tx, settings_rx) = watch::channel(store.load_settings(defaults()).await);
    let (shutdown, receiver) = watch::channel(false);
    let (claude, codex) = (Control::new(), Control::new());
    let codex_calls = Arc::new(AtomicU32::new(0));
    let scheduler = |provider, control: &Arc<Control>| Scheduler {
        provider,
        source: DataSource::Mock,
        settings: settings_rx.clone(),
        publisher: mqtt.publisher.clone(),
        cache: cache.clone(),
        control: control.clone(),
        shutdown: receiver.clone(),
    };
    let calls = codex_calls.clone();
    tokio::spawn(scheduler(Provider::Codex, &codex).run(move |_| {
        Ok(Scripted {
            inner: MockCollector::new(Provider::Codex),
            calls: calls.clone(),
            ok_calls: codex_ok_calls,
        })
    }));
    tokio::spawn(scheduler(Provider::Claude, &claude).run(|_| {
        Ok(Scripted {
            inner: MockCollector::new(Provider::Claude),
            calls: Arc::new(AtomicU32::new(0)),
            ok_calls: u32::MAX,
        })
    }));
    let state = Arc::new(AppState {
        mock: true,
        store,
        settings: settings_tx,
        device: Mutex::new(DeviceSettings::default()),
        write_lock: tokio::sync::Mutex::new(()),
        claude,
        codex,
        publisher: mqtt.publisher.clone(),
        device_setup: device_setup.then(|| DeviceSetupConfig {
            host: "192.168.1.112".into(),
            port: 1883,
            password_file: Some(root.join("mqtt-device-password")),
        }),
        web_dist_dir: dist,
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(http::serve(listener, state, receiver));
    // Keep the runtime alive for the whole test; its event loop is the publisher's consumer.
    std::mem::forget(mqtt);
    Harness {
        base,
        client: Client::new(),
        directory,
        codex_calls,
        _shutdown: shutdown,
    }
}

struct Reply {
    status: StatusCode,
    headers: reqwest::header::HeaderMap,
    text: String,
}
impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or_else(|_| panic!("not JSON: {}", self.text))
    }
}

impl Harness {
    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> Reply {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        Reply {
            status: response.status(),
            headers: response.headers().clone(),
            text: response.text().await.unwrap(),
        }
    }
    async fn get(&self, path: &str) -> Reply {
        self.send(Method::GET, path, None).await
    }
    async fn status_until(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let value = self.get("/api/status").await.json();
            if predicate(&value) {
                return value;
            }
            assert!(Instant::now() < deadline, "status never matched: {value}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    fn file(&self, name: &str) -> std::path::PathBuf {
        self.directory.path().join("cache").join(name)
    }
}

fn mode(path: &Path) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        0o600
    }
}

fn settings_body() -> Value {
    json!({
        "providers": {
            "claude": {"enabled": true, "refresh_interval_seconds": 900},
            "codex": {"enabled": true, "refresh_interval_seconds": 120},
        },
        "stale_after_seconds": 600,
        "max_backoff_seconds": 3600,
        "request_timeout_seconds": 20,
    })
}

#[tokio::test]
async fn status_reports_providers_device_and_schedule() {
    let h = start(u32::MAX, true, true).await;
    let status = h
        .status_until(|v| {
            v["providers"]["codex"]["snapshot"]["state"] == "ok"
                && v["providers"]["claude"]["snapshot"]["error_code"] == "disabled"
                && v["providers"]["codex"]["next_refresh_at"].is_i64()
        })
        .await;
    assert_eq!(status["server"], json!({"mock": true}));
    assert_eq!(status["device"], json!({"state": "unknown"}));
    let codex = &status["providers"]["codex"];
    assert_eq!(codex["enabled"], true);
    assert_eq!(codex["snapshot"]["source"], "mock");
    assert_eq!(codex["retry_after_seconds"], Value::Null);
    let now = chrono::Utc::now().timestamp();
    let next = codex["next_refresh_at"].as_i64().unwrap();
    assert!(
        (now + 290..=now + 301).contains(&next),
        "next {next} vs now {now}"
    );
    let claude = &status["providers"]["claude"];
    assert_eq!(claude["enabled"], false);
    assert_eq!(claude["snapshot"]["state"], "unavailable");
    assert_eq!(claude["next_refresh_at"], Value::Null);
    assert_eq!(
        h.get("/api/status").await.headers["cache-control"],
        "no-store"
    );
}

#[tokio::test]
async fn settings_round_trip_validate_persist_and_apply_live() {
    let h = start(u32::MAX, true, false).await;
    let initial = h.get("/api/settings").await;
    assert_eq!(initial.status, StatusCode::OK);
    assert_eq!(initial.json(), serde_json::to_value(defaults()).unwrap());
    assert!(!h.file("settings.json").exists());

    // Invalid: nothing written, nothing changed.
    let mut bad = settings_body();
    bad["providers"]["claude"]["refresh_interval_seconds"] = json!(59);
    bad["max_backoff_seconds"] = json!(30);
    let reply = h.send(Method::PUT, "/api/settings", Some(bad)).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let body = reply.json();
    assert_eq!(body["error"], "invalid");
    assert_eq!(
        body["fields"]["providers.claude.refresh_interval_seconds"],
        "must be between 60 and 3600"
    );
    assert_eq!(
        body["fields"]["max_backoff_seconds"],
        "must be between 60 and 86400"
    );
    assert!(!h.file("settings.json").exists());
    assert_eq!(h.get("/api/settings").await.json(), initial.json());

    // Valid: echoed, persisted 0600, and Claude (disabled by default) starts without restart.
    let reply = h
        .send(Method::PUT, "/api/settings", Some(settings_body()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), settings_body());
    assert_eq!(h.get("/api/settings").await.json(), settings_body());
    assert_eq!(mode(&h.file("settings.json")), 0o600);
    let status = h
        .status_until(|v| v["providers"]["claude"]["snapshot"]["state"] == "ok")
        .await;
    assert_eq!(status["providers"]["claude"]["enabled"], true);

    // Disabling publishes the disabled snapshot and clears the schedule.
    let mut off = settings_body();
    off["providers"]["codex"]["enabled"] = json!(false);
    assert_eq!(
        h.send(Method::PUT, "/api/settings", Some(off)).await.status,
        StatusCode::OK
    );
    let status = h
        .status_until(|v| v["providers"]["codex"]["snapshot"]["error_code"] == "disabled")
        .await;
    assert_eq!(status["providers"]["codex"]["next_refresh_at"], Value::Null);
}

#[tokio::test]
async fn malformed_requests_use_the_error_shape() {
    let h = start(u32::MAX, true, false).await;
    let put = |body: Value| h.send(Method::PUT, "/api/settings", Some(body));
    // Wrong types and missing fields name every offending path.
    let reply = put(json!({"providers": {"claude": {"enabled": "yes"}}})).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let fields = &reply.json()["fields"];
    assert_eq!(fields["providers.claude.enabled"], "must be a boolean");
    assert_eq!(fields["providers.codex"], "is required");
    assert_eq!(fields["stale_after_seconds"], "is required");

    let raw = |content_type: &'static str, body: Vec<u8>| {
        h.client
            .put(format!("{}/api/settings", h.base))
            .header("content-type", content_type)
            .body(body)
            .send()
    };
    let reply = raw("application/json", b"{nope".to_vec()).await.unwrap();
    assert_eq!(reply.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.json::<Value>().await.unwrap(),
        json!({"error": "invalid_json"})
    );
    let reply = raw("text/plain", settings_body().to_string().into_bytes())
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        reply.json::<Value>().await.unwrap(),
        json!({"error": "unsupported_media_type"})
    );
    let big = format!(r#"{{"pad":"{}"}}"#, "x".repeat(17 * 1024));
    let reply = raw("application/json", big.into_bytes()).await.unwrap();
    assert_eq!(reply.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        reply.json::<Value>().await.unwrap(),
        json!({"error": "payload_too_large"})
    );
    assert!(!h.file("settings.json").exists());

    let reply = h.send(Method::POST, "/api/status", None).await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(reply.json(), json!({"error": "method_not_allowed"}));
    for path in ["/api/nope", "/api/", "/api"] {
        let reply = h.get(path).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(reply.json(), json!({"error": "not_found"}), "{path}");
    }
}

#[tokio::test]
async fn refresh_statuses_and_never_bypasses_backoff() {
    // First fetch succeeds, every later one fails with `network` and arms the backoff floor.
    let h = start(1, true, false).await;
    h.status_until(|v| v["providers"]["codex"]["snapshot"]["state"] == "ok")
        .await;
    assert_eq!(h.codex_calls.load(Ordering::SeqCst), 1);

    let post = |name: &'static str| h.send(Method::POST, name, None);
    let reply = post("/api/refresh/claude").await;
    assert_eq!(
        (reply.status, reply.json()),
        (StatusCode::CONFLICT, json!({"error": "disabled"}))
    );
    let reply = post("/api/refresh/gemini").await;
    assert_eq!(
        (reply.status, reply.json()),
        (StatusCode::NOT_FOUND, json!({"error": "not_found"}))
    );

    let reply = post("/api/refresh/codex").await;
    assert_eq!(
        (reply.status, reply.json()),
        (StatusCode::ACCEPTED, json!({"scheduled": true}))
    );
    let status = h
        .status_until(|v| v["providers"]["codex"]["snapshot"]["error_code"] == "network")
        .await;
    assert_eq!(h.codex_calls.load(Ordering::SeqCst), 2);
    // Interval 300 s, first failure: backoff 600 s.
    let retry = status["providers"]["codex"]["retry_after_seconds"]
        .as_u64()
        .unwrap();
    assert!((590..=601).contains(&retry), "retry_after {retry}");

    let reply = post("/api/refresh/codex").await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    let body = reply.json();
    assert_eq!(body["error"], "rate_limited");
    assert!((590..=601).contains(&body["retry_after_seconds"].as_u64().unwrap()));
    // A shorter interval and a denied refresh both leave the floor in force.
    let mut faster = settings_body();
    faster["providers"]["codex"]["refresh_interval_seconds"] = json!(60);
    faster["max_backoff_seconds"] = json!(3600);
    assert_eq!(
        h.send(Method::PUT, "/api/settings", Some(faster))
            .await
            .status,
        StatusCode::OK
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        h.codex_calls.load(Ordering::SeqCst),
        2,
        "fetched before the backoff floor"
    );
    assert_eq!(
        post("/api/refresh/codex").await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn device_settings_validate_persist_and_default() {
    let h = start(u32::MAX, true, false).await;
    let defaults = json!({"layout": "cards", "timezone": "ICT-7", "brightness": 80});
    assert_eq!(h.get("/api/device").await.json(), defaults);
    assert!(!h.file("device.json").exists());

    for (body, field) in [
        (
            json!({"layout": "grid", "timezone": "ICT-7", "brightness": 80}),
            "layout",
        ),
        (
            json!({"layout": "cards", "timezone": "ICT 7", "brightness": 80}),
            "timezone",
        ),
        (
            json!({"layout": "cards", "timezone": "A".repeat(65), "brightness": 80}),
            "timezone",
        ),
        (
            json!({"layout": "cards", "timezone": "ICT-7", "brightness": 9}),
            "brightness",
        ),
        (
            json!({"layout": "cards", "timezone": "ICT-7", "brightness": 101}),
            "brightness",
        ),
        (
            json!({"layout": "cards", "timezone": "ICT-7"}),
            "brightness",
        ),
    ] {
        let reply = h.send(Method::PUT, "/api/device", Some(body)).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{field}");
        let body = reply.json();
        assert_eq!(body["error"], "invalid");
        assert!(body["fields"][field].is_string(), "{field}: {body}");
    }
    assert!(!h.file("device.json").exists());
    assert_eq!(h.get("/api/device").await.json(), defaults);

    for brightness in [10, 100] {
        let body = json!({"layout": "terminal", "timezone": "CET-1CEST,M3.5.0,M10.5.0/3", "brightness": brightness});
        let reply = h.send(Method::PUT, "/api/device", Some(body.clone())).await;
        assert_eq!((reply.status, reply.json()), (StatusCode::OK, body.clone()));
        assert_eq!(h.get("/api/device").await.json(), body);
    }
    assert_eq!(mode(&h.file("device.json")), 0o600);
    let saved: Value =
        serde_json::from_slice(&std::fs::read(h.file("device.json")).unwrap()).unwrap();
    assert_eq!(saved["brightness"], 100);
}

#[tokio::test]
async fn device_setup_returns_connection_values_and_404_when_unconfigured() {
    let h = start(u32::MAX, true, true).await;
    let reply = h.get("/api/device-setup").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json(),
        json!({"host": "192.168.1.112", "port": 1883, "username": "esp32",
               "password": DEVICE_PASSWORD, "tls": false})
    );
    assert_eq!(reply.headers["cache-control"], "no-store");
    // Password file vanishes: a clean error, not a panic or an empty password.
    std::fs::remove_file(h.directory.path().join("mqtt-device-password")).unwrap();
    let reply = h.get("/api/device-setup").await;
    assert_eq!(
        (reply.status, reply.json()),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "unavailable"})
        )
    );

    let h = start(u32::MAX, true, false).await;
    assert_eq!(
        h.get("/api/device-setup").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn no_route_leaks_the_server_password() {
    let h = start(u32::MAX, true, true).await;
    h.status_until(|v| v["providers"]["codex"]["snapshot"]["state"] == "ok")
        .await;
    let calls = [
        (Method::GET, "/api/status", None),
        (Method::GET, "/api/settings", None),
        (Method::PUT, "/api/settings", Some(settings_body())),
        (
            Method::PUT,
            "/api/settings",
            Some(json!({"bad": SERVER_PASSWORD})),
        ),
        (Method::POST, "/api/refresh/codex", None),
        (Method::POST, "/api/refresh/claude", None),
        (Method::GET, "/api/device", None),
        (
            Method::PUT,
            "/api/device",
            Some(json!({"layout": "x", "timezone": SERVER_PASSWORD, "brightness": 1})),
        ),
        (Method::GET, "/api/device-setup", None),
        (Method::GET, "/api/unknown", None),
        (Method::GET, "/", None),
        (Method::GET, "/settings", None),
        (Method::GET, "/assets/app-abc123.js", None),
    ];
    for (method, path, body) in calls {
        let reply = h.send(method.clone(), path, body).await;
        let headers = format!("{:?}", reply.headers);
        assert!(!reply.text.contains(SERVER_PASSWORD), "{method} {path}");
        assert!(!headers.contains(SERVER_PASSWORD), "{method} {path}");
        // The device password belongs to /api/device-setup alone.
        assert_eq!(
            reply.text.contains(DEVICE_PASSWORD),
            path == "/api/device-setup",
            "{method} {path}"
        );
    }
}

#[tokio::test]
async fn static_files_spa_fallback_and_cache_headers() {
    let h = start(u32::MAX, true, false).await;
    let index = h.get("/").await;
    assert_eq!(index.status, StatusCode::OK);
    assert!(index.text.contains("<title>admin</title>"));
    assert_eq!(index.headers["cache-control"], "no-cache");
    for path in ["/claude", "/device-setup", "/deep/link"] {
        let reply = h.get(path).await;
        assert_eq!(
            (reply.status, reply.text.as_str()),
            (StatusCode::OK, index.text.as_str()),
            "{path}"
        );
    }
    let asset = h.get("/assets/app-abc123.js").await;
    assert_eq!(asset.text, "console.log(1)");
    assert_eq!(
        asset.headers["cache-control"],
        "public, max-age=31536000, immutable"
    );
    // A missing asset falls back to HTML, which must never be cached as immutable.
    let missing = h.get("/assets/gone.js").await;
    assert_eq!(missing.headers["cache-control"], "no-cache");
    for reply in [&index, &asset, &h.get("/api/status").await] {
        assert!(!reply.headers.contains_key("access-control-allow-origin"));
    }
    assert_eq!(h.get("/api/nope").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_still_works_when_the_web_directory_is_missing() {
    let h = start(u32::MAX, false, false).await;
    assert_eq!(h.get("/api/settings").await.status, StatusCode::OK);
    assert_eq!(h.get("/").await.status, StatusCode::NOT_FOUND);
}
