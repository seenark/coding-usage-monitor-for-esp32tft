use crate::settings::{ProviderMap, ProviderSettings, ServerSettings};
use std::{env, net::SocketAddr, path::PathBuf};
use thiserror::Error;

pub struct Config {
    pub mock_mode: bool,
    pub codex_binary: PathBuf,
    pub codex_home: PathBuf,
    pub claude_config_dir: PathBuf,
    /// Environment/default Server settings; `settings.json` in `cache_dir` overrides them.
    pub defaults: ServerSettings,
    pub cache_dir: PathBuf,
    pub mqtt: MqttConfig,
    pub admin: AdminConfig,
}
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password_file: PathBuf,
    pub client_id: String,
    pub tls: bool,
    pub ca_file: Option<PathBuf>,
}
pub struct AdminConfig {
    pub listen: SocketAddr,
    pub web_dist_dir: PathBuf,
    /// `None` when DEVICE_SETUP_HOST is unset; `/api/device-setup` then answers 404.
    pub device_setup: Option<DeviceSetupConfig>,
}
#[derive(Clone)]
pub struct DeviceSetupConfig {
    pub host: String,
    pub port: u16,
    pub password_file: Option<PathBuf>,
}
#[derive(Debug, Error)]
#[error("invalid or missing configuration: {0}")]
pub struct ConfigError(pub &'static str);
/// Variable lookup, injectable so tests never touch the process environment.
struct Env<'a>(&'a dyn Fn(&str) -> Option<String>);
impl Env<'_> {
    fn string(&self, key: &'static str, default: &str) -> String {
        (self.0)(key).unwrap_or_else(|| default.to_owned())
    }
    fn boolean(&self, key: &'static str, default: bool) -> Result<bool, ConfigError> {
        match (self.0)(key).as_deref() {
            None => Ok(default),
            Some("true") => Ok(true),
            Some("false") => Ok(false),
            _ => Err(ConfigError(key)),
        }
    }
    fn number(&self, key: &'static str, default: u64) -> Result<u64, ConfigError> {
        match (self.0)(key) {
            None => Ok(default),
            Some(value) => value
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or(ConfigError(key)),
        }
    }
}
fn safe_text(key: &'static str, value: &str) -> Result<(), ConfigError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(ConfigError(key));
    }
    Ok(())
}
/// Maps a failed Server settings field back to the variable the operator has to fix.
fn environment_key(field: &str) -> &'static str {
    match field {
        "providers.claude.refresh_interval_seconds" => "CLAUDE_POLL_INTERVAL_SECONDS",
        "providers.codex.refresh_interval_seconds" => "CODEX_POLL_INTERVAL_SECONDS",
        "stale_after_seconds" => "STALE_AFTER_SECONDS",
        "max_backoff_seconds" => "MAX_BACKOFF_SECONDS",
        _ => "REQUEST_TIMEOUT_SECONDS",
    }
}
impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        // Non-Unicode values become U+FFFD so they fail validation instead of reading as unset.
        Self::from_lookup(&|key| env::var_os(key).map(|value| value.to_string_lossy().into_owned()))
    }
    pub fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let env = Env(lookup);
        let mqtt_port = env.number("MQTT_PORT", 1883)?;
        let tls = env.boolean("MQTT_TLS", false)?;
        let ca_file = lookup("MQTT_CA_FILE")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        if tls && ca_file.is_none() {
            return Err(ConfigError("MQTT_CA_FILE"));
        }
        let host = env.string("MQTT_HOST", "mosquitto");
        let username = env.string("MQTT_USERNAME", "quota-server");
        let client_id = env.string("MQTT_CLIENT_ID", "quota-server");
        safe_text("MQTT_HOST", &host)?;
        safe_text("MQTT_USERNAME", &username)?;
        if client_id.len() > 128 {
            return Err(ConfigError("MQTT_CLIENT_ID"));
        }
        safe_text("MQTT_CLIENT_ID", &client_id)?;
        let mock_mode = env.boolean("MOCK_MODE", false)?;
        let defaults = ServerSettings {
            providers: ProviderMap {
                claude: ProviderSettings {
                    // Mock mode always demonstrates both providers until settings.json says otherwise.
                    enabled: env.boolean("CLAUDE_PRIVATE_USAGE_ENABLED", false)? || mock_mode,
                    refresh_interval_seconds: env.number("CLAUDE_POLL_INTERVAL_SECONDS", 600)?,
                },
                codex: ProviderSettings {
                    enabled: env.boolean("CODEX_ENABLED", true)?,
                    refresh_interval_seconds: env.number("CODEX_POLL_INTERVAL_SECONDS", 300)?,
                },
            },
            stale_after_seconds: env.number("STALE_AFTER_SECONDS", 900)?,
            max_backoff_seconds: env.number("MAX_BACKOFF_SECONDS", 1800)?,
            request_timeout_seconds: env.number("REQUEST_TIMEOUT_SECONDS", 12)?,
        };
        if let Err(fields) = defaults.validate() {
            // A too-small backoff is a symptom when another field is also out of range.
            let field = fields
                .keys()
                .find(|field| *field != "max_backoff_seconds")
                .or(fields.keys().next())
                .map_or("", String::as_str);
            return Err(ConfigError(environment_key(field)));
        }
        let listen = env
            .string("ADMIN_LISTEN", "127.0.0.1:8080")
            .parse()
            .map_err(|_| ConfigError("ADMIN_LISTEN"))?;
        let device_setup = match lookup("DEVICE_SETUP_HOST") {
            None => None,
            Some(host) => {
                safe_text("DEVICE_SETUP_HOST", &host)?;
                Some(DeviceSetupConfig {
                    host,
                    port: u16::try_from(env.number("DEVICE_SETUP_PORT", 1883)?)
                        .map_err(|_| ConfigError("DEVICE_SETUP_PORT"))?,
                    password_file: lookup("DEVICE_PASSWORD_FILE")
                        .filter(|path| !path.is_empty())
                        .map(PathBuf::from),
                })
            }
        };
        Ok(Self {
            mock_mode,
            codex_binary: env.string("CODEX_BINARY", "codex").into(),
            codex_home: env.string("CODEX_HOME", "/run/auth/codex").into(),
            claude_config_dir: env.string("CLAUDE_CONFIG_DIR", "/run/auth/claude").into(),
            defaults,
            cache_dir: env.string("CACHE_DIR", "/var/lib/quota-server").into(),
            mqtt: MqttConfig {
                host,
                port: u16::try_from(mqtt_port).map_err(|_| ConfigError("MQTT_PORT"))?,
                username,
                password_file: env
                    .string("MQTT_PASSWORD_FILE", "/run/secrets/mqtt-server-password")
                    .into(),
                client_id,
                tls,
                ca_file,
            },
            admin: AdminConfig {
                listen,
                web_dist_dir: env.string("WEB_DIST_DIR", "/app/web").into(),
                device_setup,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        Config::from_lookup(&|key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    #[test]
    fn defaults_apply_without_environment() {
        let config = load(&[]).unwrap();
        let settings = config.defaults;
        assert_eq!(settings.providers.claude.refresh_interval_seconds, 600);
        assert_eq!(settings.providers.codex.refresh_interval_seconds, 300);
        assert!(!settings.providers.claude.enabled && settings.providers.codex.enabled);
        assert!(config.admin.device_setup.is_none());
        assert_eq!(config.admin.listen.to_string(), "127.0.0.1:8080");
    }
    #[test]
    fn environment_overrides_default() {
        let config = load(&[
            ("CLAUDE_POLL_INTERVAL_SECONDS", "900"),
            ("CODEX_POLL_INTERVAL_SECONDS", "60"),
            ("CLAUDE_PRIVATE_USAGE_ENABLED", "true"),
            ("CODEX_ENABLED", "false"),
            ("DEVICE_SETUP_HOST", "192.168.1.112"),
            ("DEVICE_SETUP_PORT", "1884"),
        ])
        .unwrap();
        let settings = config.defaults;
        assert_eq!(settings.providers.claude.refresh_interval_seconds, 900);
        assert_eq!(settings.providers.codex.refresh_interval_seconds, 60);
        assert!(settings.providers.claude.enabled && !settings.providers.codex.enabled);
        let setup = config.admin.device_setup.unwrap();
        assert_eq!((setup.host.as_str(), setup.port), ("192.168.1.112", 1884));
    }
    #[test]
    fn out_of_range_environment_names_its_variable() {
        for (key, value) in [
            ("CLAUDE_POLL_INTERVAL_SECONDS", "59"),
            ("CODEX_POLL_INTERVAL_SECONDS", "3601"),
            ("STALE_AFTER_SECONDS", "59"),
            ("REQUEST_TIMEOUT_SECONDS", "61"),
            ("MAX_BACKOFF_SECONDS", "86401"),
            // Backoff below the default Claude interval of 600.
            ("MAX_BACKOFF_SECONDS", "300"),
        ] {
            assert_eq!(load(&[(key, value)]).err().unwrap().0, key, "{key}={value}");
        }
        assert_eq!(
            load(&[("ADMIN_LISTEN", "nope")]).err().unwrap().0,
            "ADMIN_LISTEN"
        );
    }
    #[test]
    fn mock_mode_enables_claude_by_default() {
        assert!(
            load(&[("MOCK_MODE", "true")])
                .unwrap()
                .defaults
                .providers
                .claude
                .enabled
        );
    }
}
