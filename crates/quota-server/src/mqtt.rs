use crate::config::MqttConfig;
use quota_core::{DEVICE_CONFIG_TOPIC, DEVICE_STATUS_TOPIC, Provider, SERVER_STATUS_TOPIC};
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS, Transport};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::task::JoinHandle;

const ONLINE: &[u8] = br#"{"state":"online"}"#;
const OFFLINE: &[u8] = br#"{"state":"offline"}"#;

/// Latest retained payloads plus publications that could not be queued yet.
/// Never held across an await.
#[derive(Default)]
struct Outbox {
    retained: HashMap<Provider, Vec<u8>>,
    pending: HashSet<Provider>,
    online_pending: bool,
    device_config: Option<Vec<u8>>,
    device_config_pending: bool,
    subscribe_pending: bool,
    device_state: DeviceState,
}

/// What the server knows about the device, learned only from `device/status`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    #[default]
    Unknown,
    Online,
    Offline,
}
impl DeviceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Online => "online",
            Self::Offline => "offline",
        }
    }
    /// Anything but the two documented payloads is ignored, never treated as offline.
    pub fn parse(payload: &[u8]) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
        match value.get("state")?.as_str()? {
            "online" => Some(Self::Online),
            "offline" => Some(Self::Offline),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Publisher {
    client: AsyncClient,
    outbox: Arc<Mutex<Outbox>>,
}
pub struct Runtime {
    pub publisher: Publisher,
    task: JoinHandle<()>,
}
impl Runtime {
    pub async fn new(config: MqttConfig) -> Result<Self, &'static str> {
        let password = tokio::fs::read_to_string(&config.password_file)
            .await
            .map_err(|_| "MQTT password file unavailable")?;
        let password = password.trim_end_matches(['\r', '\n']);
        if password.is_empty() || password.len() > 1024 || password.contains(['\r', '\n', '\0']) {
            return Err("MQTT password file invalid");
        }
        let mut options = MqttOptions::new(config.client_id, config.host, config.port);
        options.set_credentials(config.username, password);
        options.set_keep_alive(Duration::from_secs(30));
        options.set_last_will(LastWill::new(
            SERVER_STATUS_TOPIC,
            br#"{"state":"offline"}"#.to_vec(),
            QoS::AtLeastOnce,
            true,
        ));
        if config.tls {
            let ca = tokio::fs::read(config.ca_file.ok_or("MQTT CA file required")?)
                .await
                .map_err(|_| "MQTT CA file unavailable")?;
            options.set_transport(Transport::tls(ca, None, None));
        }
        let (client, mut eventloop) = AsyncClient::new(options, 32);
        let outbox = Arc::new(Mutex::new(Outbox::default()));
        let publisher = Publisher {
            client: client.clone(),
            outbox: outbox.clone(),
        };
        let task = tokio::spawn(async move {
            let mut attempts = 0u32;
            loop {
                match eventloop.poll().await {
                    Ok(Event::Outgoing(rumqttc::Outgoing::Disconnect)) => break,
                    Ok(event) => {
                        match event {
                            Event::Incoming(Packet::ConnAck(_)) => {
                                attempts = 0;
                                let mut outbox = outbox.lock().unwrap_or_else(|e| e.into_inner());
                                outbox.online_pending = true;
                                outbox.subscribe_pending = true;
                                outbox.device_config_pending = outbox.device_config.is_some();
                                outbox.pending = outbox.retained.keys().copied().collect();
                                tracing::info!("MQTT connected");
                            }
                            Event::Incoming(Packet::Publish(message))
                                if message.topic == DEVICE_STATUS_TOPIC =>
                            {
                                if let Some(state) = DeviceState::parse(&message.payload) {
                                    let mut outbox =
                                        outbox.lock().unwrap_or_else(|e| e.into_inner());
                                    outbox.device_state = state;
                                }
                            }
                            _ => {}
                        }
                        // Never await queue capacity here: this task is the queue's consumer.
                        flush(&client, &outbox);
                    }
                    Err(_) => {
                        attempts = attempts.saturating_add(1);
                        tracing::warn!("MQTT disconnected; reconnecting");
                        let delay = 1u64 << attempts.min(5);
                        tokio::time::sleep(
                            Duration::from_secs(delay)
                                + Duration::from_millis(rand::random_range(0..=250)),
                        )
                        .await;
                    }
                }
            }
        });
        Ok(Self { publisher, task })
    }
    /// Bounded: a broker outage must not block process termination.
    pub async fn shutdown(mut self) {
        let client = &self.publisher.client;
        let _ = client.try_publish(
            SERVER_STATUS_TOPIC,
            QoS::AtLeastOnce,
            true,
            OFFLINE.to_vec(),
        );
        let _ = client.try_disconnect();
        if tokio::time::timeout(Duration::from_secs(5), &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
        }
    }
}

fn flush(client: &AsyncClient, outbox: &Mutex<Outbox>) {
    let mut outbox = outbox.lock().unwrap_or_else(|e| e.into_inner());
    if outbox.online_pending {
        outbox.online_pending = client
            .try_publish(SERVER_STATUS_TOPIC, QoS::AtLeastOnce, true, ONLINE.to_vec())
            .is_err();
    }
    if outbox.subscribe_pending {
        outbox.subscribe_pending = client
            .try_subscribe(DEVICE_STATUS_TOPIC, QoS::AtLeastOnce)
            .is_err();
    }
    if outbox.device_config_pending {
        if let Some(bytes) = outbox.device_config.clone() {
            outbox.device_config_pending = client
                .try_publish(DEVICE_CONFIG_TOPIC, QoS::AtLeastOnce, true, bytes)
                .is_err();
        }
    }
    let pending = std::mem::take(&mut outbox.pending);
    for provider in pending {
        let Some(bytes) = outbox.retained.get(&provider).cloned() else {
            continue;
        };
        if client
            .try_publish(provider.topic(), QoS::AtLeastOnce, true, bytes)
            .is_err()
        {
            outbox.pending.insert(provider);
        }
    }
}

impl Publisher {
    /// Bytes are a validated normalized snapshot, encoded once by the scheduler.
    /// Never waits for queue capacity; a full queue defers delivery to the event loop,
    /// which always republishes the latest snapshot for that provider.
    pub async fn publish(&self, provider: Provider, bytes: Vec<u8>) -> Result<(), &'static str> {
        let mut outbox = self.outbox.lock().unwrap_or_else(|e| e.into_inner());
        outbox.retained.insert(provider, bytes.clone());
        if self
            .client
            .try_publish(provider.topic(), QoS::AtLeastOnce, true, bytes)
            .is_err()
        {
            outbox.pending.insert(provider);
            return Err("MQTT publication deferred");
        }
        outbox.pending.remove(&provider);
        Ok(())
    }
    /// Latest device config wins; same deferral rules as provider snapshots.
    pub fn publish_device_config(&self, bytes: Vec<u8>) -> Result<(), &'static str> {
        let mut outbox = self.outbox.lock().unwrap_or_else(|e| e.into_inner());
        outbox.device_config = Some(bytes.clone());
        outbox.device_config_pending = self
            .client
            .try_publish(DEVICE_CONFIG_TOPIC, QoS::AtLeastOnce, true, bytes)
            .is_err();
        if outbox.device_config_pending {
            return Err("MQTT publication deferred");
        }
        Ok(())
    }
    pub fn device_state(&self) -> DeviceState {
        self.outbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .device_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_status_payloads() {
        assert_eq!(
            DeviceState::parse(br#"{"state":"online"}"#),
            Some(DeviceState::Online)
        );
        assert_eq!(
            DeviceState::parse(br#"{"state":"offline"}"#),
            Some(DeviceState::Offline)
        );
        for bad in [
            &b""[..],
            b"online",
            br#"{"state":"sleeping"}"#,
            br#"{"state":1}"#,
            br#"{}"#,
        ] {
            assert_eq!(DeviceState::parse(bad), None);
        }
    }
}
