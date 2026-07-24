//! Hardware-security-gated storage for OAuth credentials.
//!
//! This module deliberately exposes no NVS or ESP-IDF types to callers. A board
//! adapter supplies the runtime security checks and an atomic storage adapter
//! supplies the encrypted-NVS operations. Token records do not implement
//! `Debug`, and this boundary never logs their contents.

use std::time::SystemTime;

/// The exact credential fields retained after OAuth completion.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretTokenRecord {
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
    pub account_id: Option<String>,
    pub access_token_expires_at: SystemTime,
    pub last_refresh_at: SystemTime,
}

/// Runtime checks required before credentials may be read or written.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SecurityStatus {
    pub secure_boot: bool,
    pub flash_encryption: bool,
    pub nvs_encryption: bool,
}

impl SecurityStatus {
    pub const fn verified() -> Self {
        Self { secure_boot: true, flash_encryption: true, nvs_encryption: true }
    }
    pub const fn is_verified(self) -> bool {
        self.secure_boot && self.flash_encryption && self.nvs_encryption
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SecretsError {
    Security(SecurityStatus),
    Storage,
    Verification,
}

pub type SecretsResult<T> = Result<T, SecretsError>;

/// Runtime security provider. On ESP-IDF this must query efuse/runtime state,
/// not compile-time configuration alone.
pub trait SecurityProvider {
    fn status(&self) -> SecurityStatus;
}

/// Atomic encrypted-NVS adapter. Implementations must ensure `save` is a
/// single-record commit (no partially visible token fields).
pub trait SecretBackend {
    fn load(&mut self) -> Result<Option<SecretTokenRecord>, SecretsError>;
    fn save(&mut self, record: &SecretTokenRecord) -> Result<(), SecretsError>;
    fn erase(&mut self) -> Result<(), SecretsError>;
    fn contains(&mut self, key: &str) -> Result<bool, SecretsError>;
    fn erase_key(&mut self, key: &str) -> Result<(), SecretsError>;
}

/// Application-owned NVS keys erased by factory reset. Keep this inventory in
/// one place so reset can verify that no application data remains.
pub const APPLICATION_KEYS: &[&str] = &[
    "settings",
    "wifi",
    "touch_calibration",
    "cached_usage",
    "oauth_state",
    "credentials",
];

pub struct SecretsStore<B, P> {
    backend: B,
    security: P,
}

impl<B, P> SecretsStore<B, P>
where
    B: SecretBackend,
    P: SecurityProvider,
{
    pub fn new(backend: B, security: P) -> Self { Self { backend, security } }

    pub fn security_status(&self) -> SecurityStatus { self.security.status() }

    fn require_security(&self) -> SecretsResult<()> {
        let status = self.security.status();
        if status.is_verified() { Ok(()) } else { Err(SecretsError::Security(status)) }
    }

    /// Read credentials only from a verified encrypted-NVS boundary.
    pub fn load(&mut self) -> SecretsResult<Option<SecretTokenRecord>> {
        self.require_security()?;
        self.backend.load()
    }

    /// Atomically commit credentials. No backend operation is attempted when
    /// runtime security checks fail.
    pub fn save(&mut self, record: &SecretTokenRecord) -> SecretsResult<()> {
        self.require_security()?;
        self.backend.save(record)
    }


    /// Erase temporary secret bytes immediately after an operation. Callers
    /// should use this for serialized HTTP/form buffers and response buffers.
    pub fn zero_temporary_bytes(bytes: &mut [u8]) {
        for byte in bytes {
            // Volatile writes keep the wipe observable in optimized builds.
            unsafe { std::ptr::write_volatile(byte, 0); }
        }
        std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
    }

    /// Logout always erases the local record; remote revocation belongs to the
    /// auth transport and must happen before calling this method.
    pub fn logout_erase(&mut self) -> SecretsResult<()> {
        self.require_security()?;
        self.backend.erase()
    }

    /// Backwards-friendly spelling for callers that only need local erase.
    pub fn erase(&mut self) -> SecretsResult<()> {
        self.logout_erase()
    }
    /// Factory reset clears every application-owned key and verifies absence.
    /// Security checks still apply because this operation addresses encrypted
    /// NVS and must never silently fall back to an insecure backend.
    pub fn factory_reset(&mut self) -> SecretsResult<()> {
        self.require_security()?;
        for &key in APPLICATION_KEYS {
            self.backend.erase_key(key)?;
        }
        for &key in APPLICATION_KEYS {
            if self.backend.contains(key)? { return Err(SecretsError::Verification); }
        }
        Ok(())
    }

    pub fn into_parts(self) -> (B, P) { (self.backend, self.security) }
}
