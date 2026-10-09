//! Offline by default. Invoke only against a dedicated authenticated test broker:
//! MQTT_TEST_HOST=127.0.0.1 MQTT_TEST_PORT=<port>
//! MQTT_TEST_SERVER_PASSWORD_FILE=<generated mqtt-server-password>
//! MQTT_TEST_DEVICE_PASSWORD_FILE=<generated mqtt-device-password>
//! cargo test -p quota-server --test mqtt_integration -- --ignored --test-threads=1
//! Usernames default to quota-server/esp32; MQTT_TEST_{SERVER,DEVICE}_USERNAME overrides them.
//! The restart test also requires MQTT_TEST_RESTART_DIR, managed by scripts/smoke.sh.
//! Never point these tests at a live broker: they overwrite the three retained test topics.

use std::{collections::VecDeque, path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use quota_core::{
    BankedResets, DEVICE_CONFIG_TOPIC, DEVICE_STATUS_TOPIC, DataSource, DetailsStatus, ErrorCode,
    Provider, ProviderState, SERVER_STATUS_TOPIC, Snapshot, Window, decode_snapshot,
};
use quota_server::{
    cache::Cache,
    config::MqttConfig,
    mqtt::{DeviceState, Runtime},
    providers::{Collector, ProviderError},
    scheduler::{Control, Scheduler},
    settings::{DeviceSettings, Layout, ProviderMap, ProviderSettings, ServerSettings},
};
use rumqttc::{
    AsyncClient, ConnectReturnCode, Event, EventLoop, MqttOptions, Packet, Publish, QoS,
};
use tokio::{
    sync::{Notify, watch},
    time::{Instant, timeout},
};

const WAIT: Duration = Duration::from_secs(20);

struct Environment {
    host: String,
    port: u16,
    server_username: String,
    device_username: String,
    server_password: PathBuf,
    device_password: PathBuf,
}

impl Environment {
    fn load() -> Self {
        fn required(name: &str) -> String {
            std::env::var(name).unwrap_or_else(|_| panic!("Missing {name}; use scripts/smoke.sh"))
        }
        Self {
            host: required("MQTT_TEST_HOST"),
            port: required("MQTT_TEST_PORT")
                .parse()
                .expect("Invalid test broker port"),
            server_username: std::env::var("MQTT_TEST_SERVER_USERNAME")
                .unwrap_or_else(|_| "quota-server".into()),
            device_username: std::env::var("MQTT_TEST_DEVICE_USERNAME")
                .unwrap_or_else(|_| "esp32".into()),
            server_password: required("MQTT_TEST_SERVER_PASSWORD_FILE").into(),
            device_password: required("MQTT_TEST_DEVICE_PASSWORD_FILE").into(),
        }
    }

    fn runtime_config(&self) -> MqttConfig {
        MqttConfig {
            host: self.host.clone(),
            port: self.port,
            username: self.server_username.clone(),
            password_file: self.server_password.clone(),
            client_id: client_id("publisher"),
            tls: false,
            ca_file: None,
        }
    }

    async fn connection(&self, server: bool) -> Connection {
        let password = tokio::fs::read_to_string(if server {
            &self.server_password
        } else {
            &self.device_password
        })
        .await
        .expect("Generated MQTT password file unavailable");
        let mut options = MqttOptions::new(
            client_id(if server { "server" } else { "device" }),
            &self.host,
            self.port,
        );
        options.set_credentials(
            if server {
                &self.server_username
            } else {
                &self.device_username
            },
            password.trim(),
        );
        options.set_keep_alive(Duration::from_secs(5));
        let (client, eventloop) = AsyncClient::new(options, 32);
        let mut connection = Connection {
            client,
            eventloop,
            pending: VecDeque::new(),
        };
        connection.connected().await;
        connection
    }
}

fn client_id(role: &str) -> String {
    format!(
        "quota-test-{role}-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    )
}

struct Connection {
    client: AsyncClient,
    eventloop: EventLoop,
    pending: VecDeque<Publish>,
}

impl Connection {
    async fn packet(&mut self, deadline: Instant) -> Packet {
        loop {
            let result = timeout(
                deadline.saturating_duration_since(Instant::now()),
                self.eventloop.poll(),
            )
            .await
            .expect("MQTT packet deadline exceeded");
            match result {
                Ok(Event::Incoming(packet)) => return packet,
                Ok(_) => {}
                Err(_) => {
                    assert!(Instant::now() < deadline, "MQTT connection unavailable");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }

    async fn connected(&mut self) {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Packet::ConnAck(ack) = self.packet(deadline).await {
                assert_eq!(
                    ack.code,
                    ConnectReturnCode::Success,
                    "Test MQTT authentication rejected"
                );
                return;
            }
        }
    }

    async fn subscribe(&mut self, topic: &str) {
        self.client
            .subscribe(topic, QoS::AtLeastOnce)
            .await
            .expect("Subscription queue unavailable");
        let deadline = Instant::now() + WAIT;
        loop {
            match self.packet(deadline).await {
                Packet::SubAck(_) => return,
                Packet::Publish(message) => self.pending.push_back(message),
                _ => {}
            }
        }
    }

    async fn next(&mut self, deadline: Instant) -> Publish {
        if let Some(message) = self.pending.pop_front() {
            return message;
        }
        loop {
            if let Packet::Publish(message) = self.packet(deadline).await {
                return message;
            }
        }
    }

    async fn snapshot(
        &mut self,
        provider: Provider,
        predicate: impl Fn(&Snapshot) -> bool,
    ) -> (Snapshot, bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            let message = self.next(deadline).await;
            if message.topic == provider.topic() {
                let snapshot = decode_snapshot(&message.topic, &message.payload)
                    .expect("Invalid snapshot received");
                assert_eq!(
                    snapshot.source,
                    DataSource::Mock,
                    "Test broker contains non-mock data"
                );
                if predicate(&snapshot) {
                    return (snapshot, message.retain);
                }
            }
        }
    }

    async fn status(&mut self, state: &str) -> bool {
        let deadline = Instant::now() + WAIT;
        loop {
            let message = self.next(deadline).await;
            if message.topic == SERVER_STATUS_TOPIC {
                let value: serde_json::Value =
                    serde_json::from_slice(&message.payload).expect("Invalid server status");
                if value["state"] == state {
                    return message.retain;
                }
            }
        }
    }

    /// Waits for `expected` on `topic`; earlier retained values from other tests are skipped.
    async fn expect_payload(&mut self, topic: &str, expected: &serde_json::Value) {
        let deadline = Instant::now() + WAIT;
        loop {
            let message = self.next(deadline).await;
            if message.topic == topic
                && serde_json::from_slice::<serde_json::Value>(&message.payload)
                    .ok()
                    .as_ref()
                    == Some(expected)
            {
                return;
            }
        }
    }

    async fn acknowledged_publish(&mut self, topic: &str, payload: Vec<u8>) {
        self.client
            .publish(topic, QoS::AtLeastOnce, true, payload)
            .await
            .expect("Publication queue unavailable");
        let deadline = Instant::now() + WAIT;
        loop {
            match self.packet(deadline).await {
                Packet::PubAck(_) => return,
                Packet::Publish(message) => self.pending.push_back(message),
                _ => {}
            }
        }
    }
}

fn fixture(provider: Provider, fetched_at: i64, used: f64) -> Snapshot {
    let mut snapshot = Snapshot::empty(provider, DataSource::Mock, fetched_at);
    snapshot.state = ProviderState::Ok;
    snapshot.fetched_at = Some(fetched_at);
    snapshot.windows = vec![
        Window::new(
            "mock-fixture",
            "primary",
            "Mock expired window",
            Some(300),
            Some(used),
            Some(900),
        )
        .unwrap(),
        Window::new(
            "mock-fixture",
            "unknown",
            "Mock unknown quota",
            None,
            None,
            None,
        )
        .unwrap(),
    ];
    if provider == Provider::Codex {
        snapshot.banked_resets = Some(BankedResets {
            available_count: Some(3),
            details_status: DetailsStatus::Partial,
            known_expirations_at: vec![900],
            known_nonexpiring_count: 0,
        });
    }
    snapshot
}

async fn publish(runtime: &Runtime, snapshot: &Snapshot) {
    runtime
        .publisher
        .publish(snapshot.provider, serde_json::to_vec(snapshot).unwrap())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "Requires dedicated authenticated broker and generated test password files"]
async fn actual_mock_binary_delivers_retained_snapshots_on_first_subscription_and_reconnect() {
    let environment = Environment::load();
    let cache = tempfile::tempdir().unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_quota-server"))
        .env_clear()
        .env("MOCK_MODE", "true")
        .env("CODEX_ENABLED", "true")
        .env("CLAUDE_PRIVATE_USAGE_ENABLED", "false")
        .env("CLAUDE_POLL_INTERVAL_SECONDS", "3600")
        .env("CODEX_POLL_INTERVAL_SECONDS", "3600")
        .env("MAX_BACKOFF_SECONDS", "3600")
        .env("ADMIN_LISTEN", "127.0.0.1:0")
        .env("MQTT_HOST", &environment.host)
        .env("MQTT_PORT", environment.port.to_string())
        .env("MQTT_USERNAME", &environment.server_username)
        .env("MQTT_PASSWORD_FILE", &environment.server_password)
        .env("MQTT_CLIENT_ID", client_id("binary"))
        .env("CACHE_DIR", cache.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("Mock server binary unavailable");
    // Cache creation proves this binary fetched, rather than accepting a previous test's retained data.
    timeout(WAIT, async {
        loop {
            assert!(
                child.try_wait().unwrap().is_none(),
                "Mock server exited before publishing"
            );
            if cache.path().join("codex.json").exists() && cache.path().join("claude.json").exists()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("Mock snapshots were not persisted");
    let stored = Cache::new(cache.path()).await.unwrap();
    let mut expected = Vec::new();
    let mut readiness = environment.connection(false).await;
    for provider in [Provider::Codex, Provider::Claude] {
        let snapshot = stored
            .load(provider, DataSource::Mock)
            .await
            .unwrap()
            .unwrap();
        readiness.subscribe(provider.topic()).await;
        readiness
            .snapshot(provider, |received| *received == snapshot)
            .await;
        expected.push(snapshot);
    }
    drop(readiness);
    let mut first = environment.connection(false).await;
    first.subscribe(SERVER_STATUS_TOPIC).await;
    first.status("online").await;
    let mut originals = Vec::new();
    for expected_snapshot in expected {
        let provider = expected_snapshot.provider;
        first.subscribe(provider.topic()).await;
        let (snapshot, retained) = first
            .snapshot(provider, |snapshot| *snapshot == expected_snapshot)
            .await;
        assert!(retained, "First subscription must deliver retained data");
        assert_eq!(snapshot.windows.len(), 2);
        originals.push(snapshot);
    }
    drop(first);
    let mut reconnect = environment.connection(false).await;
    for original in originals {
        reconnect.subscribe(original.provider.topic()).await;
        let (snapshot, retained) = reconnect
            .snapshot(original.provider, |snapshot| {
                snapshot.state == ProviderState::Ok
            })
            .await;
        assert!(retained);
        assert_eq!(
            snapshot, original,
            "Reconnect must not invent quota or advance fetched_at"
        );
    }
    // SIGKILL does not send MQTT DISCONNECT, so this also tests the actual binary's LWT.
    let mut status = environment.connection(false).await;
    status.subscribe(SERVER_STATUS_TOPIC).await;
    status.status("online").await;
    child.kill().await.unwrap();
    timeout(WAIT, child.wait()).await.unwrap().unwrap();
    status.status("offline").await;
}

#[tokio::test]
#[ignore = "Requires dedicated authenticated broker and generated test password files"]
async fn duplicate_snapshots_preserve_quota_and_acl_enforces_device_read_server_write() {
    let environment = Environment::load();
    let runtime = Runtime::new(environment.runtime_config()).await.unwrap();
    let original = fixture(Provider::Codex, 1000, 62.0);
    let mut observer = environment.connection(false).await;
    let options = MqttOptions::new(client_id("anonymous"), &environment.host, environment.port);
    let (_anonymous_client, mut anonymous_loop) = AsyncClient::new(options, 4);
    let anonymous = timeout(WAIT, async {
        loop {
            match anonymous_loop.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                    break ack.code != ConnectReturnCode::Success;
                }
                Err(_) => break true,
                _ => {}
            }
        }
    })
    .await
    .expect("Anonymous MQTT authentication deadline exceeded");
    assert!(anonymous, "Broker allowed unauthenticated connection");
    observer.subscribe(Provider::Codex.topic()).await;
    publish(&runtime, &original).await;
    let (first, _) = observer
        .snapshot(Provider::Codex, |snapshot| {
            snapshot.fetched_at == Some(1000)
        })
        .await;
    publish(&runtime, &original).await;
    let (duplicate, _) = observer
        .snapshot(Provider::Codex, |snapshot| {
            snapshot.fetched_at == Some(1000)
        })
        .await;
    assert_eq!(first, duplicate);
    assert_eq!(duplicate.windows[0].remaining_percent, Some(38.0));
    assert_eq!(duplicate.windows[1].remaining_percent, None);
    assert_eq!(
        duplicate.banked_resets.as_ref().unwrap().available_count,
        Some(3)
    );

    let mut device_writer = environment.connection(false).await;
    let forbidden = fixture(Provider::Codex, 9999, 99.0);
    device_writer
        .acknowledged_publish(
            Provider::Codex.topic(),
            serde_json::to_vec(&forbidden).unwrap(),
        )
        .await;
    device_writer
        .acknowledged_publish(
            SERVER_STATUS_TOPIC,
            br#"{"state":"mock-forbidden"}"#.to_vec(),
        )
        .await;
    // MQTT 3 brokers may acknowledge a denied write. Check externally visible retained state.
    let mut retained = environment.connection(false).await;
    retained.subscribe(Provider::Codex.topic()).await;
    let (unchanged, is_retained) = retained.snapshot(Provider::Codex, |_| true).await;
    assert!(is_retained);
    assert_eq!(
        unchanged, original,
        "Read-only device changed retained quota"
    );
    let claude_original = fixture(Provider::Claude, 1100, 28.0);
    let mut claude_observer = environment.connection(false).await;
    claude_observer.subscribe(Provider::Claude.topic()).await;
    publish(&runtime, &claude_original).await;
    claude_observer
        .snapshot(Provider::Claude, |snapshot| {
            snapshot.fetched_at == Some(1100)
        })
        .await;
    let claude_forbidden = fixture(Provider::Claude, 9999, 99.0);
    device_writer
        .acknowledged_publish(
            Provider::Claude.topic(),
            serde_json::to_vec(&claude_forbidden).unwrap(),
        )
        .await;
    let mut claude_retained = environment.connection(false).await;
    claude_retained.subscribe(Provider::Claude.topic()).await;
    let (claude_unchanged, claude_is_retained) =
        claude_retained.snapshot(Provider::Claude, |_| true).await;
    assert!(claude_is_retained);
    assert_eq!(
        claude_unchanged, claude_original,
        "Read-only device changed Claude quota"
    );
    retained.subscribe(SERVER_STATUS_TOPIC).await;
    let status = retained.next(Instant::now() + WAIT).await;
    assert_eq!(status.topic, SERVER_STATUS_TOPIC);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&status.payload).unwrap()["state"],
        "online"
    );

    let mut server_reader = environment.connection(true).await;
    server_reader.subscribe("ai-quota/v1/#").await;
    publish(&runtime, &original).await;
    observer
        .snapshot(Provider::Codex, |snapshot| {
            snapshot.fetched_at == Some(1000)
        })
        .await;
    assert!(
        timeout(
            Duration::from_millis(750),
            server_reader.next(Instant::now() + WAIT)
        )
        .await
        .is_err(),
        "Publish-only server received provider data"
    );
    // Read access to status must be denied too, not merely provider snapshots.
    server_reader.subscribe(SERVER_STATUS_TOPIC).await;
    assert!(
        timeout(
            Duration::from_millis(750),
            server_reader.next(Instant::now() + WAIT)
        )
        .await
        .is_err(),
        "Publish-only server received status data"
    );
    runtime.shutdown().await;
}

async fn wait_device_state(runtime: &Runtime, expected: DeviceState) {
    timeout(WAIT, async {
        while runtime.publisher.device_state() != expected {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("Server never saw device state {expected:?}"));
}

#[tokio::test]
#[ignore = "Requires dedicated authenticated broker and generated test password files"]
async fn device_config_and_status_follow_acl_and_server_tracks_device_state() {
    let environment = Environment::load();
    let runtime = Runtime::new(environment.runtime_config()).await.unwrap();
    let config = DeviceSettings {
        layout: Layout::Gauges,
        timezone: "UTC0".into(),
        brightness: 55,
    };
    let expected = serde_json::json!({
        "schema_version": 1, "layout": "gauges", "timezone": "UTC0", "brightness": 55
    });
    let _ = runtime
        .publisher
        .publish_device_config(config.mqtt_payload());
    // Device reads the retained config on its first subscription.
    let mut device = environment.connection(false).await;
    device.subscribe(DEVICE_CONFIG_TOPIC).await;
    device.expect_payload(DEVICE_CONFIG_TOPIC, &expected).await;
    // Device may not write config; the retained value must not change.
    device
        .acknowledged_publish(
            DEVICE_CONFIG_TOPIC,
            br#"{"schema_version":1,"layout":"terminal","timezone":"X","brightness":10}"#.to_vec(),
        )
        .await;
    let mut observer = environment.connection(false).await;
    observer.subscribe(DEVICE_CONFIG_TOPIC).await;
    observer
        .expect_payload(DEVICE_CONFIG_TOPIC, &expected)
        .await;
    // Device status: only the device writes it, and the server tracks it.
    device
        .acknowledged_publish(DEVICE_STATUS_TOPIC, br#"{"state":"offline"}"#.to_vec())
        .await;
    wait_device_state(&runtime, DeviceState::Offline).await;
    device
        .acknowledged_publish(DEVICE_STATUS_TOPIC, br#"{"state":"online"}"#.to_vec())
        .await;
    wait_device_state(&runtime, DeviceState::Online).await;
    let mut server = environment.connection(true).await;
    server
        .acknowledged_publish(DEVICE_STATUS_TOPIC, br#"{"state":"offline"}"#.to_vec())
        .await;
    // Only the server's side may read status: the device account has write access alone.
    server.subscribe(DEVICE_STATUS_TOPIC).await;
    // The retained message is delivered first; a server overwrite would make this time out.
    server
        .expect_payload(DEVICE_STATUS_TOPIC, &serde_json::json!({"state": "online"}))
        .await;
    // Server reads status but never config.
    server.subscribe(DEVICE_CONFIG_TOPIC).await;
    assert!(
        timeout(
            Duration::from_millis(750),
            server.next(Instant::now() + WAIT)
        )
        .await
        .is_err(),
        "Server received device config"
    );
    // Leave no retained status: later tests assert the server sees no unexpected topics.
    device
        .acknowledged_publish(DEVICE_STATUS_TOPIC, Vec::new())
        .await;
    runtime.shutdown().await;
}

struct FailingCollector {
    calls: u32,
    fail: Arc<Notify>,
}
impl Collector for FailingCollector {
    async fn fetch(&mut self) -> Result<Snapshot, ProviderError> {
        self.calls += 1;
        match self.calls {
            1 => Ok(fixture(Provider::Codex, 1000, 62.0)),
            2 => {
                self.fail.notified().await;
                Err(ProviderError::new(ErrorCode::Network))
            }
            _ => std::future::pending().await,
        }
    }
}

struct ContinuingCollector {
    calls: i64,
}
impl Collector for ContinuingCollector {
    async fn fetch(&mut self) -> Result<Snapshot, ProviderError> {
        self.calls += 1;
        Ok(fixture(Provider::Claude, 2000 + self.calls, 28.0))
    }
}

#[tokio::test]
#[ignore = "Requires dedicated authenticated broker and generated test password files"]
async fn scheduler_failure_keeps_original_fetch_time_and_other_provider_continues() {
    let environment = Environment::load();
    let runtime = Runtime::new(environment.runtime_config()).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::new(directory.path()).await.unwrap();
    let provider = |seconds| ProviderSettings {
        enabled: true,
        refresh_interval_seconds: seconds,
    };
    // Held for the whole test: dropping the sender stops every scheduler.
    let (_settings_tx, settings) = watch::channel(ServerSettings {
        providers: ProviderMap {
            claude: provider(1),
            codex: provider(1),
        },
        stale_after_seconds: 900,
        max_backoff_seconds: 1,
        request_timeout_seconds: 12,
    });
    let (shutdown, receiver) = watch::channel(false);
    let failure = Arc::new(Notify::new());
    let mut codex = environment.connection(false).await;
    let mut claude = environment.connection(false).await;
    codex.subscribe(Provider::Codex.topic()).await;
    claude.subscribe(Provider::Claude.topic()).await;
    let scheduler_for = |provider| Scheduler {
        provider,
        source: DataSource::Mock,
        settings: settings.clone(),
        publisher: runtime.publisher.clone(),
        cache: cache.clone(),
        control: Control::new(),
        shutdown: receiver.clone(),
    };
    let fail = failure.clone();
    let codex_task = tokio::spawn(scheduler_for(Provider::Codex).run(move |_| {
        Ok(FailingCollector {
            calls: 0,
            fail: fail.clone(),
        })
    }));
    let claude_task =
        tokio::spawn(scheduler_for(Provider::Claude).run(|_| Ok(ContinuingCollector { calls: 0 })));
    let (success, _) = codex
        .snapshot(Provider::Codex, |snapshot| {
            snapshot.state == ProviderState::Ok
                && snapshot.fetched_at == Some(1000)
                && snapshot.published_at > 1000
        })
        .await;
    let (independent, _) = claude
        .snapshot(Provider::Claude, |snapshot| {
            snapshot.fetched_at.is_some_and(|time| time >= 2002)
        })
        .await;
    failure.notify_one();
    let (failed, _) = codex
        .snapshot(Provider::Codex, |snapshot| {
            snapshot.error_code == Some(ErrorCode::Network)
        })
        .await;
    assert_eq!(failed.fetched_at, success.fetched_at);
    assert!(failed.published_at >= success.published_at && failed.published_at > 1000);
    assert_ne!(failed.state, ProviderState::Ok);
    assert_eq!(failed.windows, success.windows);
    assert_eq!(failed.banked_resets, success.banked_resets);
    assert_eq!(
        failed.windows[0].remaining_percent,
        Some(38.0),
        "Expired fixture must not reset quota"
    );
    assert_eq!(failed.windows[1].remaining_percent, None);
    let (continuing, _) = claude
        .snapshot(Provider::Claude, |snapshot| {
            snapshot.fetched_at > independent.fetched_at
        })
        .await;
    assert_eq!(continuing.state, ProviderState::Ok);
    let persisted = cache
        .load(Provider::Codex, DataSource::Mock)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.fetched_at, Some(1000));
    assert_eq!(
        persisted.state,
        ProviderState::Ok,
        "Failure must not replace last-success cache"
    );
    shutdown.send(true).unwrap();
    timeout(WAIT, codex_task).await.unwrap().unwrap();
    timeout(WAIT, claude_task).await.unwrap().unwrap();
    runtime.shutdown().await;
}

async fn marker(directory: &std::path::Path, name: &str) {
    tokio::fs::write(directory.join(name), b"mock test orchestration\n")
        .await
        .unwrap();
}

async fn wait_marker(directory: &std::path::Path, name: &str) {
    timeout(Duration::from_secs(60), async {
        while !directory.join(name).exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("Smoke orchestration marker {name} not received"));
}

#[tokio::test]
#[ignore = "Requires scripts/smoke.sh broker restart orchestration"]
async fn broker_restart_preserves_retained_data_and_publisher_reconnects_with_latest_snapshot() {
    let environment = Environment::load();
    let directory = PathBuf::from(
        std::env::var_os("MQTT_TEST_RESTART_DIR")
            .expect("Run restart acceptance with scripts/smoke.sh"),
    );
    let original = fixture(Provider::Codex, 3000, 62.0);
    let runtime = Runtime::new(environment.runtime_config()).await.unwrap();
    let mut observer = environment.connection(false).await;
    observer.subscribe(Provider::Codex.topic()).await;
    publish(&runtime, &original).await;
    observer
        .snapshot(Provider::Codex, |snapshot| {
            snapshot.fetched_at == Some(3000)
        })
        .await;
    let mut status = environment.connection(false).await;
    status.subscribe(SERVER_STATUS_TOPIC).await;
    status.status("online").await;
    runtime.shutdown().await;
    status.status("offline").await;
    drop(observer);
    drop(status);
    // No publisher survives this restart: only the broker's disk persistence can deliver the data.
    marker(&directory, "persistence-ready").await;
    wait_marker(&directory, "persistence-restarted").await;
    let mut persisted = environment.connection(false).await;
    persisted.subscribe(Provider::Codex.topic()).await;
    let (restored, retained) = persisted.snapshot(Provider::Codex, |_| true).await;
    assert!(retained);
    assert_eq!(restored, original);
    persisted.subscribe(SERVER_STATUS_TOPIC).await;
    assert!(persisted.status("offline").await);
    drop(persisted);

    let runtime = Runtime::new(environment.runtime_config()).await.unwrap();
    let mut before = environment.connection(false).await;
    before.subscribe(Provider::Codex.topic()).await;
    let initial = fixture(Provider::Codex, 4000, 76.0);
    publish(&runtime, &initial).await;
    before
        .snapshot(Provider::Codex, |snapshot| {
            snapshot.fetched_at == Some(4000)
        })
        .await;
    drop(before);
    marker(&directory, "reconnect-ready").await;
    wait_marker(&directory, "broker-stopped").await;
    let latest = fixture(Provider::Codex, 5000, 83.0);
    publish(&runtime, &latest).await;
    marker(&directory, "publication-queued").await;
    wait_marker(&directory, "reconnect-restarted").await;
    let mut reconnect = environment.connection(false).await;
    reconnect.subscribe(Provider::Codex.topic()).await;
    let (delivered, _) = reconnect
        .snapshot(Provider::Codex, |snapshot| {
            snapshot.fetched_at == Some(5000)
        })
        .await;
    assert_eq!(delivered, latest);
    let mut retained_reader = environment.connection(false).await;
    retained_reader.subscribe(Provider::Codex.topic()).await;
    let (retained_latest, retained) = retained_reader.snapshot(Provider::Codex, |_| true).await;
    assert!(retained);
    assert_eq!(retained_latest, latest);
    retained_reader.subscribe(SERVER_STATUS_TOPIC).await;
    retained_reader.status("online").await;
    runtime.shutdown().await;
}
