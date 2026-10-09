//! File-only Claude OAuth credentials. No home-directory or Keychain fallback.

use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use quota_core::ErrorCode;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::providers::ProviderError;

// One account per server. Hold this lock across reading, HTTP refresh, and writing.
// This also serializes separate collectors in the same process.
pub(crate) static CLAUDE_WRITER: Mutex<()> = Mutex::const_new(());
const MAX_CREDENTIAL_BYTES: u64 = 65_536;

pub(crate) struct Credentials {
    raw: Value,
    metadata: Metadata,
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) expires_at_ms: Option<i64>,
}

fn invalid() -> ProviderError {
    ProviderError::new(ErrorCode::InvalidResponse)
}

fn file_error(error: std::io::Error) -> ProviderError {
    ProviderError::new(if error.kind() == std::io::ErrorKind::NotFound {
        ErrorCode::Unauthenticated
    } else {
        ErrorCode::Cache
    })
}

fn token(value: Option<&Value>) -> Result<Option<String>, ProviderError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(Some(text.trim().to_owned())),
        _ => Err(invalid()),
    }
}

fn same_file(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

fn private_file(metadata: &Metadata) -> bool {
    metadata.is_file() && metadata.permissions().mode() & 0o777 == 0o600
}

impl Credentials {
    pub(crate) fn load(config_dir: &Path) -> Result<Self, ProviderError> {
        // The supplied isolated directory is the only credential source. Reject
        // symlink directories/files and insecure permissions before reading bytes.
        let directory = fs::symlink_metadata(config_dir).map_err(file_error)?;
        if !directory.is_dir() || directory.permissions().mode() & 0o077 != 0 {
            return Err(ProviderError::new(ErrorCode::Cache));
        }
        let path = config_dir.join(".credentials.json");
        let before = fs::symlink_metadata(&path).map_err(file_error)?;
        if !private_file(&before) {
            return Err(ProviderError::new(ErrorCode::Cache));
        }
        let file = File::open(&path).map_err(file_error)?;
        let metadata = file.metadata().map_err(file_error)?;
        if !same_file(&before, &metadata) || !private_file(&metadata) {
            return Err(ProviderError::new(ErrorCode::Cache));
        }
        if metadata.len() > MAX_CREDENTIAL_BYTES {
            return Err(invalid());
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_CREDENTIAL_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(file_error)?;
        if bytes.len() as u64 > MAX_CREDENTIAL_BYTES {
            return Err(invalid());
        }
        let raw: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let oauth = raw
            .get("claudeAiOauth")
            .and_then(Value::as_object)
            .ok_or_else(invalid)?;
        let access = token(oauth.get("accessToken"))?
            .ok_or_else(|| ProviderError::new(ErrorCode::Unauthenticated))?;
        let access_token = if access
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bearer "))
        {
            access[7..].trim().to_owned()
        } else {
            access
        };
        if access_token.is_empty() {
            return Err(ProviderError::new(ErrorCode::Unauthenticated));
        }
        let refresh_token = token(oauth.get("refreshToken"))?;
        let expires_at_ms = match oauth.get("expiresAt") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_i64()
                    .filter(|value| *value >= 0)
                    .ok_or_else(invalid)?,
            ),
        };
        let scopes = oauth
            .get("scopes")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::new(ErrorCode::AccessDenied))?;
        if !scopes
            .iter()
            .any(|scope| scope.as_str() == Some("user:profile"))
        {
            return Err(ProviderError::new(ErrorCode::AccessDenied));
        }
        Ok(Self {
            raw,
            metadata,
            access_token,
            refresh_token,
            expires_at_ms,
        })
    }

    pub(crate) fn persist_refresh(
        mut self,
        config_dir: &Path,
        access_token: String,
        refresh_token: Option<String>,
        expires_at_ms: i64,
    ) -> Result<Self, ProviderError> {
        let oauth = self
            .raw
            .get_mut("claudeAiOauth")
            .and_then(Value::as_object_mut)
            .ok_or_else(invalid)?;
        oauth.insert("accessToken".into(), Value::String(access_token.clone()));
        // OAuth servers may omit an unchanged refresh token. Never erase it.
        if let Some(refreshed) = &refresh_token {
            oauth.insert("refreshToken".into(), Value::String(refreshed.clone()));
        }
        oauth.insert("expiresAt".into(), Value::from(expires_at_ms));
        let mut bytes = serde_json::to_vec(&self.raw).map_err(|_| invalid())?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_CREDENTIAL_BYTES {
            return Err(invalid());
        }
        let path = config_dir.join(".credentials.json");
        let current = fs::symlink_metadata(&path).map_err(file_error)?;
        if !same_file(&current, &self.metadata) || !private_file(&current) {
            return Err(ProviderError::new(ErrorCode::Cache));
        }
        // create_new plus a random same-directory name gives atomic, private writes.
        let temporary_path =
            config_dir.join(format!(".credentials-{:032x}.tmp", rand::random::<u128>()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary_path)
            .map_err(file_error)?;
        let temporary = TemporaryCredential(temporary_path);
        let created = file.metadata().map_err(file_error)?;
        if created.uid() != self.metadata.uid() || created.gid() != self.metadata.gid() {
            std::os::unix::fs::chown(
                &temporary.0,
                Some(self.metadata.uid()),
                Some(self.metadata.gid()),
            )
            .map_err(file_error)?;
        }
        file.set_permissions(fs::Permissions::from_mode(
            self.metadata.permissions().mode() & 0o777,
        ))
        .map_err(file_error)?;
        file.write_all(&bytes).map_err(file_error)?;
        file.sync_all().map_err(file_error)?;
        // Detect a concurrent external writer rather than overwrite its session.
        let current = fs::symlink_metadata(&path).map_err(file_error)?;
        if !same_file(&current, &self.metadata) || !private_file(&current) {
            return Err(ProviderError::new(ErrorCode::Cache));
        }
        fs::rename(&temporary.0, &path).map_err(file_error)?;
        File::open(config_dir)
            .and_then(|directory| directory.sync_all())
            .map_err(file_error)?;
        self.metadata = file.metadata().map_err(file_error)?;
        self.access_token = access_token;
        if refresh_token.is_some() {
            self.refresh_token = refresh_token;
        }
        self.expires_at_ms = Some(expires_at_ms);
        Ok(self)
    }
}

struct TemporaryCredential(PathBuf);

impl Drop for TemporaryCredential {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
