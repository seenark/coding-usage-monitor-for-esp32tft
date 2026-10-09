use quota_core::{DataSource, Provider};
use quota_server::{
    cache::Cache,
    config::{Config, DeviceSetupConfig},
    http::{self, AppState},
    mock::MockCollector,
    mqtt,
    providers::{claude::ClaudeCollector, codex::CodexCollector},
    scheduler::{Control, Scheduler},
    settings::Store,
};
use std::{sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::watch};

#[tokio::main]
async fn main() {
    // Provider/library debug output may contain credentials. Only our safe events are enabled.
    tracing_subscriber::fmt()
        .with_env_filter("quota_server=info,rumqttc=off,reqwest=off,hyper=off")
        .with_target(false)
        .init();
    if let Err(error) = serve().await {
        tracing::error!(code = error, "Server startup failed");
        std::process::exit(1);
    }
}
async fn serve() -> Result<(), &'static str> {
    let config = Config::from_env().map_err(|error| error.0)?;
    let cache = Cache::new(&config.cache_dir).await.map_err(|_| "cache")?;
    let store = Store::new(&config.cache_dir);
    let settings = store.load_settings(config.defaults).await;
    let device = store.load_device().await;
    let listener = TcpListener::bind(config.admin.listen)
        .await
        .map_err(|_| "ADMIN_LISTEN")?;
    let mqtt = mqtt::Runtime::new(config.mqtt).await?;
    // Retained config must reflect device.json after every restart, not wait for the next PUT.
    let _ = mqtt.publisher.publish_device_config(device.mqtt_payload());
    let (settings_tx, settings_rx) = watch::channel(settings);
    let (shutdown, receiver) = watch::channel(false);
    let source = if config.mock_mode {
        DataSource::Mock
    } else {
        DataSource::Provider
    };
    let (claude, codex) = (Control::new(), Control::new());
    let scheduler = |provider: Provider, control: &Arc<Control>| Scheduler {
        provider,
        source,
        settings: settings_rx.clone(),
        publisher: mqtt.publisher.clone(),
        cache: cache.clone(),
        control: control.clone(),
        shutdown: receiver.clone(),
    };
    let mut tasks = Vec::with_capacity(3);
    if config.mock_mode {
        for (provider, control) in [(Provider::Codex, &codex), (Provider::Claude, &claude)] {
            tasks.push(tokio::spawn(
                scheduler(provider, control).run(move |_| Ok(MockCollector::new(provider))),
            ));
        }
    } else {
        let (binary, home) = (config.codex_binary, config.codex_home);
        tasks.push(tokio::spawn(scheduler(Provider::Codex, &codex).run(
            move |timeout: Duration| Ok(CodexCollector::new(binary.clone(), home.clone(), timeout)),
        )));
        let directory = config.claude_config_dir;
        tasks.push(tokio::spawn(scheduler(Provider::Claude, &claude).run(
            move |timeout: Duration| ClaudeCollector::new(directory.clone(), timeout),
        )));
    }
    log_device_setup(config.admin.device_setup.as_ref());
    let state = Arc::new(AppState {
        mock: config.mock_mode,
        store,
        settings: settings_tx,
        device: std::sync::Mutex::new(device),
        write_lock: tokio::sync::Mutex::new(()),
        claude,
        codex,
        publisher: mqtt.publisher.clone(),
        device_setup: config.admin.device_setup,
        web_dist_dir: config.admin.web_dist_dir,
    });
    tasks.push(tokio::spawn(async move {
        if http::serve(listener, state, receiver).await.is_err() {
            // Dropping the state would silently stop every scheduler; restart instead.
            tracing::error!("Admin HTTP server stopped");
            std::process::exit(1);
        }
    }));
    tracing::info!(
        mode = if config.mock_mode { "mock" } else { "provider" },
        "Quota server started; provider authentication is independent of readiness"
    );
    wait_for_signal().await;
    let _ = shutdown.send(true);
    for task in tasks {
        let _ = tokio::time::timeout(Duration::from_secs(10), task).await;
    }
    mqtt.shutdown().await;
    Ok(())
}

/// Prints the values typed into the device's setup form. Opt-in: only when the
/// compose file supplies DEVICE_SETUP_HOST. The device account is read/limited-write on the
/// device and quota topics and LAN-only, so its 12-digit password is logged on purpose.
/// Timezone is not part of the form; it arrives from the Admin app via `device/config`.
fn log_device_setup(setup: Option<&DeviceSetupConfig>) {
    let Some(setup) = setup else {
        return;
    };
    let password = setup
        .password_file
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|value| value.trim().to_owned())
        .unwrap_or_else(|| "(unreadable)".into());
    tracing::info!(
        "Device setup form: MQTT host={} port={} user=esp32 password={password} transport=plain TCP",
        setup.host,
        setup.port
    );
}
async fn wait_for_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _ = signal.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
