//! Server settings and Device settings: validation, JSON files in CACHE_DIR.
//! Precedence for Server settings: settings.json > environment > built-in default.
use crate::cache::write_atomic;
use quota_core::Provider;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, io, path::PathBuf};

/// Field path -> human readable reason. Serialized as the `fields` object of a 400 response.
pub type FieldErrors = BTreeMap<String, String>;

pub const INTERVAL_RANGE: (u64, u64) = (60, 3600);
pub const STALE_RANGE: (u64, u64) = (60, 86_400);
pub const BACKOFF_RANGE: (u64, u64) = (60, 86_400);
pub const TIMEOUT_RANGE: (u64, u64) = (1, 60);
pub const BRIGHTNESS_RANGE: (u64, u64) = (10, 100);
const MAX_FILE_BYTES: u64 = 16 * 1024;
const SETTINGS_FILE: &str = "settings.json";
const DEVICE_FILE: &str = "device.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderSettings {
    pub enabled: bool,
    pub refresh_interval_seconds: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderMap {
    pub claude: ProviderSettings,
    pub codex: ProviderSettings,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerSettings {
    pub providers: ProviderMap,
    pub stale_after_seconds: u64,
    pub max_backoff_seconds: u64,
    pub request_timeout_seconds: u64,
}

fn range_error(fields: &mut FieldErrors, path: &str, value: u64, (low, high): (u64, u64)) {
    if value < low || value > high {
        fields.insert(path.into(), format!("must be between {low} and {high}"));
    }
}

/// Reads `object[key]` as an unsigned integer. Negative numbers are returned as 0 so the
/// range check, not the type check, reports them.
fn integer(object: &Value, key: &str, path: &str, fields: &mut FieldErrors) -> Option<u64> {
    match object.get(key) {
        None | Some(Value::Null) => {
            fields.insert(path.into(), "is required".into());
            None
        }
        Some(Value::Number(number)) if number.is_i64() || number.is_u64() => {
            Some(number.as_u64().unwrap_or(0))
        }
        Some(_) => {
            fields.insert(path.into(), "must be an integer".into());
            None
        }
    }
}

fn boolean(object: &Value, key: &str, path: &str, fields: &mut FieldErrors) -> Option<bool> {
    match object.get(key) {
        None | Some(Value::Null) => {
            fields.insert(path.into(), "is required".into());
            None
        }
        Some(Value::Bool(value)) => Some(*value),
        Some(_) => {
            fields.insert(path.into(), "must be a boolean".into());
            None
        }
    }
}

fn object<'a>(value: &'a Value, key: &str, path: &str, fields: &mut FieldErrors) -> &'a Value {
    static EMPTY: Value = Value::Null;
    match value.get(key) {
        Some(inner) if inner.is_object() => inner,
        None | Some(Value::Null) => {
            fields.insert(path.into(), "is required".into());
            &EMPTY
        }
        Some(_) => {
            fields.insert(path.into(), "must be an object".into());
            &EMPTY
        }
    }
}

fn provider_from_json(
    value: &Value,
    name: &str,
    fields: &mut FieldErrors,
) -> Option<ProviderSettings> {
    let path = format!("providers.{name}");
    let value = object(value, name, &path, fields);
    if value.is_null() {
        return None;
    }
    let enabled = boolean(value, "enabled", &format!("{path}.enabled"), fields);
    let interval = integer(
        value,
        "refresh_interval_seconds",
        &format!("{path}.refresh_interval_seconds"),
        fields,
    );
    Some(ProviderSettings {
        enabled: enabled?,
        refresh_interval_seconds: interval?,
    })
}

impl ServerSettings {
    pub fn provider(&self, provider: Provider) -> &ProviderSettings {
        match provider {
            Provider::Claude => &self.providers.claude,
            Provider::Codex => &self.providers.codex,
        }
    }

    /// Strict full-replace parse. Unknown keys are ignored; every known key is required.
    pub fn from_json(value: &Value) -> Result<Self, FieldErrors> {
        let mut fields = FieldErrors::new();
        let providers = object(value, "providers", "providers", &mut fields);
        let claude = provider_from_json(providers, "claude", &mut fields);
        let codex = provider_from_json(providers, "codex", &mut fields);
        let stale = integer(
            value,
            "stale_after_seconds",
            "stale_after_seconds",
            &mut fields,
        );
        let backoff = integer(
            value,
            "max_backoff_seconds",
            "max_backoff_seconds",
            &mut fields,
        );
        let timeout = integer(
            value,
            "request_timeout_seconds",
            "request_timeout_seconds",
            &mut fields,
        );
        let (Some(claude), Some(codex), Some(stale), Some(backoff), Some(timeout)) =
            (claude, codex, stale, backoff, timeout)
        else {
            return Err(fields);
        };
        let settings = Self {
            providers: ProviderMap { claude, codex },
            stale_after_seconds: stale,
            max_backoff_seconds: backoff,
            request_timeout_seconds: timeout,
        };
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<(), FieldErrors> {
        let mut fields = FieldErrors::new();
        for (name, provider) in [
            ("claude", &self.providers.claude),
            ("codex", &self.providers.codex),
        ] {
            range_error(
                &mut fields,
                &format!("providers.{name}.refresh_interval_seconds"),
                provider.refresh_interval_seconds,
                INTERVAL_RANGE,
            );
        }
        range_error(
            &mut fields,
            "stale_after_seconds",
            self.stale_after_seconds,
            STALE_RANGE,
        );
        range_error(
            &mut fields,
            "max_backoff_seconds",
            self.max_backoff_seconds,
            BACKOFF_RANGE,
        );
        range_error(
            &mut fields,
            "request_timeout_seconds",
            self.request_timeout_seconds,
            TIMEOUT_RANGE,
        );
        let longest = self
            .providers
            .claude
            .refresh_interval_seconds
            .max(self.providers.codex.refresh_interval_seconds);
        if !fields.contains_key("max_backoff_seconds") && self.max_backoff_seconds < longest {
            fields.insert(
                "max_backoff_seconds".into(),
                format!("must be at least the longest refresh interval ({longest})"),
            );
        }
        if fields.is_empty() {
            Ok(())
        } else {
            Err(fields)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    Cards,
    Gauges,
    Terminal,
}
impl Layout {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "cards" => Some(Self::Cards),
            "gauges" => Some(Self::Gauges),
            "terminal" => Some(Self::Terminal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceSettings {
    pub layout: Layout,
    pub timezone: String,
    pub brightness: u8,
}
impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            layout: Layout::Cards,
            timezone: "ICT-7".into(),
            brightness: 80,
        }
    }
}
impl DeviceSettings {
    pub fn from_json(value: &Value) -> Result<Self, FieldErrors> {
        let mut fields = FieldErrors::new();
        let layout = match value.get("layout") {
            None | Some(Value::Null) => {
                fields.insert("layout".into(), "is required".into());
                None
            }
            Some(Value::String(text)) => {
                let layout = Layout::parse(text);
                if layout.is_none() {
                    fields.insert(
                        "layout".into(),
                        "must be one of cards, gauges, terminal".into(),
                    );
                }
                layout
            }
            Some(_) => {
                fields.insert("layout".into(), "must be a string".into());
                None
            }
        };
        let timezone = match value.get("timezone") {
            None | Some(Value::Null) => {
                fields.insert("timezone".into(), "is required".into());
                None
            }
            Some(Value::String(text)) if valid_timezone(text) => Some(text.clone()),
            Some(Value::String(_)) => {
                fields.insert(
                    "timezone".into(),
                    "must be 1 to 64 characters from A-Z a-z 0-9 + - : , . / < >".into(),
                );
                None
            }
            Some(_) => {
                fields.insert("timezone".into(), "must be a string".into());
                None
            }
        };
        let brightness = integer(value, "brightness", "brightness", &mut fields);
        if let Some(brightness) = brightness {
            range_error(&mut fields, "brightness", brightness, BRIGHTNESS_RANGE);
        }
        match (layout, timezone, brightness) {
            (Some(layout), Some(timezone), Some(brightness)) if fields.is_empty() => Ok(Self {
                layout,
                timezone,
                brightness: u8::try_from(brightness).unwrap_or(u8::MAX),
            }),
            _ => Err(fields),
        }
    }

    /// Retained payload on `ai-quota/v1/device/config`; field rules live in docs/MQTT_PROTOCOL.md.
    pub fn mqtt_payload(&self) -> Vec<u8> {
        let mut object = Map::new();
        object.insert("schema_version".into(), json!(1));
        if let Value::Object(rest) = serde_json::to_value(self).unwrap_or_default() {
            object.extend(rest);
        }
        serde_json::to_vec(&object).unwrap_or_default()
    }
}

pub fn valid_timezone(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+-:,./<>".contains(&byte))
}

/// JSON files in CACHE_DIR. Both files are always written whole, atomically, mode 0600.
#[derive(Clone)]
pub struct Store {
    directory: PathBuf,
}
impl Store {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    /// File value wins; a missing or invalid file falls back to `defaults` (env, then built-in).
    pub async fn load_settings(&self, defaults: ServerSettings) -> ServerSettings {
        self.load(SETTINGS_FILE, ServerSettings::from_json)
            .await
            .unwrap_or(defaults)
    }
    pub async fn load_device(&self) -> DeviceSettings {
        self.load(DEVICE_FILE, DeviceSettings::from_json)
            .await
            .unwrap_or_default()
    }
    pub async fn save_settings(&self, settings: &ServerSettings) -> io::Result<()> {
        self.save(SETTINGS_FILE, settings).await
    }
    pub async fn save_device(&self, device: &DeviceSettings) -> io::Result<()> {
        self.save(DEVICE_FILE, device).await
    }

    async fn load<T>(&self, name: &str, parse: fn(&Value) -> Result<T, FieldErrors>) -> Option<T> {
        let path = self.directory.join(name);
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(_) => {
                tracing::warn!(file = name, "Settings file unreadable; using defaults");
                return None;
            }
        };
        let value = if metadata.len() > MAX_FILE_BYTES {
            None
        } else {
            tokio::fs::read(&path)
                .await
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        };
        let parsed = value.and_then(|value| parse(&value).ok());
        if parsed.is_none() {
            tracing::warn!(file = name, "Settings file invalid; using defaults");
        }
        parsed
    }

    async fn save<T: Serialize>(&self, name: &str, value: &T) -> io::Result<()> {
        let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
        write_atomic(&self.directory, name, &bytes).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn with(mutate: impl FnOnce(&mut Value)) -> Value {
        let mut value = serde_json::to_value(defaults()).unwrap();
        mutate(&mut value);
        value
    }
    fn failing(value: Value) -> FieldErrors {
        ServerSettings::from_json(&value).unwrap_err()
    }

    #[test]
    fn interval_boundaries() {
        for (seconds, ok) in [(59, false), (60, true), (3600, true), (3601, false)] {
            let value = with(|v| {
                v["providers"]["claude"]["refresh_interval_seconds"] = json!(seconds);
                v["max_backoff_seconds"] = json!(3600);
            });
            let result = ServerSettings::from_json(&value);
            assert_eq!(result.is_ok(), ok, "interval {seconds}");
            if !ok {
                assert_eq!(
                    result.unwrap_err()["providers.claude.refresh_interval_seconds"],
                    "must be between 60 and 3600"
                );
            }
        }
    }
    #[test]
    fn other_numeric_boundaries() {
        for (key, low, high) in [
            ("stale_after_seconds", 60, 86_400),
            ("max_backoff_seconds", 60, 86_400),
            ("request_timeout_seconds", 1, 60),
        ] {
            // Keep max_backoff >= intervals while probing its lower bound.
            let base = |value: u64| {
                with(|v| {
                    v["providers"]["claude"]["refresh_interval_seconds"] = json!(60);
                    v["providers"]["codex"]["refresh_interval_seconds"] = json!(60);
                    v[key] = json!(value);
                })
            };
            assert!(ServerSettings::from_json(&base(low)).is_ok(), "{key} low");
            assert!(ServerSettings::from_json(&base(high)).is_ok(), "{key} high");
            assert!(failing(base(low - 1)).contains_key(key), "{key} below");
            assert!(failing(base(high + 1)).contains_key(key), "{key} above");
        }
    }
    #[test]
    fn max_backoff_must_cover_every_interval() {
        let value = with(|v| {
            v["providers"]["codex"]["refresh_interval_seconds"] = json!(3000);
            v["max_backoff_seconds"] = json!(2999);
        });
        assert!(failing(value).contains_key("max_backoff_seconds"));
        let equal = with(|v| {
            v["providers"]["codex"]["refresh_interval_seconds"] = json!(3000);
            v["max_backoff_seconds"] = json!(3000);
        });
        assert!(ServerSettings::from_json(&equal).is_ok());
    }
    #[test]
    fn types_and_missing_fields_report_each_path() {
        let errors = failing(
            json!({"providers": {"claude": {"enabled": "yes"}}, "stale_after_seconds": 1.5}),
        );
        assert_eq!(errors["providers.claude.enabled"], "must be a boolean");
        assert_eq!(
            errors["providers.claude.refresh_interval_seconds"],
            "is required"
        );
        assert_eq!(errors["providers.codex"], "is required");
        assert_eq!(errors["stale_after_seconds"], "must be an integer");
        assert_eq!(errors["max_backoff_seconds"], "is required");
        assert!(ServerSettings::from_json(&json!([])).is_err());
        let negative = with(|v| v["stale_after_seconds"] = json!(-5));
        assert!(failing(negative).contains_key("stale_after_seconds"));
    }

    fn device(layout: Value, timezone: Value, brightness: Value) -> Value {
        json!({"layout": layout, "timezone": timezone, "brightness": brightness})
    }
    #[test]
    fn device_brightness_boundaries() {
        for (brightness, ok) in [(9, false), (10, true), (100, true), (101, false)] {
            let result = DeviceSettings::from_json(&device(
                json!("cards"),
                json!("ICT-7"),
                json!(brightness),
            ));
            assert_eq!(result.is_ok(), ok, "brightness {brightness}");
        }
        assert!(
            DeviceSettings::from_json(&device(json!("cards"), json!("ICT-7"), json!(80.5)))
                .is_err()
        );
        assert!(
            DeviceSettings::from_json(&device(json!("cards"), json!("ICT-7"), json!("80")))
                .is_err()
        );
    }
    #[test]
    fn device_layout_and_timezone_rules() {
        for layout in ["cards", "gauges", "terminal"] {
            assert!(
                DeviceSettings::from_json(&device(json!(layout), json!("UTC0"), json!(50))).is_ok()
            );
        }
        for layout in [json!("Cards"), json!("grid"), json!(""), json!(3)] {
            let errors =
                DeviceSettings::from_json(&device(layout, json!("UTC0"), json!(50))).unwrap_err();
            assert!(errors.contains_key("layout"));
        }
        for timezone in [
            "CET-1CEST,M3.5.0,M10.5.0/3",
            "<+0330>-3:30",
            "EST5EDT,M3.2.0,M11.1.0",
        ] {
            assert!(
                DeviceSettings::from_json(&device(json!("cards"), json!(timezone), json!(50)))
                    .is_ok()
            );
        }
        let too_long = "A".repeat(65);
        for timezone in [
            "",
            "ICT 7",
            "Asia/Bangkok;",
            "UTC\n",
            "ไทย",
            too_long.as_str(),
        ] {
            let errors =
                DeviceSettings::from_json(&device(json!("cards"), json!(timezone), json!(50)))
                    .unwrap_err();
            assert!(errors.contains_key("timezone"), "{timezone:?}");
        }
        assert!(valid_timezone(&"A".repeat(64)));
    }
    #[test]
    fn device_mqtt_payload_matches_protocol() {
        let payload: Value =
            serde_json::from_slice(&DeviceSettings::default().mqtt_payload()).unwrap();
        assert_eq!(
            payload,
            json!({"schema_version": 1, "layout": "cards", "timezone": "ICT-7", "brightness": 80})
        );
    }

    #[tokio::test]
    async fn atomic_private_persistence_and_precedence() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path());
        // No file: environment/default value is used.
        assert_eq!(store.load_settings(defaults()).await, defaults());
        assert_eq!(store.load_device().await, DeviceSettings::default());
        // File wins over the supplied defaults.
        let mut saved = defaults();
        saved.providers.claude.enabled = true;
        saved.providers.claude.refresh_interval_seconds = 900;
        store.save_settings(&saved).await.unwrap();
        let mut other = defaults();
        other.providers.codex.refresh_interval_seconds = 61;
        assert_eq!(store.load_settings(other).await, saved);
        let device = DeviceSettings {
            layout: Layout::Terminal,
            timezone: "UTC0".into(),
            brightness: 10,
        };
        store.save_device(&device).await.unwrap();
        assert_eq!(Store::new(directory.path()).load_device().await, device);
        // Only the two final files remain: temp files are renamed away.
        let mut names: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["device.json", "settings.json"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in names {
                let mode = std::fs::metadata(directory.path().join(name))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o777, 0o600);
            }
        }
    }
    #[tokio::test]
    async fn invalid_or_oversized_files_fall_back_to_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path());
        for content in [
            b"not json".to_vec(),
            br#"{"providers":{}}"#.to_vec(),
            vec![b' '; 20_000],
        ] {
            std::fs::write(directory.path().join("settings.json"), &content).unwrap();
            assert_eq!(store.load_settings(defaults()).await, defaults());
        }
        std::fs::write(
            directory.path().join("device.json"),
            br#"{"layout":"cards","timezone":"ICT-7","brightness":5}"#,
        )
        .unwrap();
        assert_eq!(store.load_device().await, DeviceSettings::default());
    }
}
