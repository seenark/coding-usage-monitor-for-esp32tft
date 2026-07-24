//! Experimental OpenAI Codex device authentication boundary.
//!
//! This module deliberately contains all OAuth URLs and DTOs. Callers receive
//! redacted, categorized errors and never need to handle provider wire types.

use std::time::{Duration, SystemTime};

pub const OPENAI_CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const DEVICE_USERCODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
pub const DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
pub const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const OAUTH_REVOKE_URL: &str = "https://auth.openai.com/oauth/revoke";
pub const VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthErrorKind { Transport, Rejected, Expired, InvalidResponse, Security, Storage }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthError { pub kind: AuthErrorKind, pub message: &'static str }
impl AuthError { fn new(kind: AuthErrorKind, message: &'static str) -> Self { Self { kind, message } } }
pub type AuthResult<T> = Result<T, AuthError>;

#[derive(Clone, PartialEq, Eq)]
pub struct TokenRecord { pub id_token: String, pub access_token: String, pub refresh_token: String, pub account_id: Option<String>, pub access_token_expires_at: SystemTime, pub last_refresh_at: SystemTime }

#[derive(Clone, PartialEq, Eq)]
pub struct DevicePrompt { pub user_code: String, pub verification_url: &'static str }
#[derive(Clone, PartialEq, Eq)]
pub struct PkceState { pub verifier: String, pub challenge: String }

pub trait AuthTransport { fn post_json(&mut self, url: &str, body: &str) -> AuthResult<String>; fn post_form(&mut self, url: &str, body: &str) -> AuthResult<String>; }
pub trait AuthStorage { fn load(&mut self) -> AuthResult<Option<TokenRecord>>; fn save(&mut self, token: &TokenRecord) -> AuthResult<()>; fn erase(&mut self) -> AuthResult<()>; }
pub trait PkceProvider { fn generate(&mut self) -> AuthResult<PkceState>; }
pub trait SecurityCheck { fn secure(&self) -> bool; }

#[derive(serde::Deserialize)] struct UserCodeDto { device_auth_id: String, user_code: String, #[serde(default)] interval: Option<u64> }
#[derive(serde::Deserialize)] struct DeviceTokenDto { authorization_code: String, #[serde(default)] id_token: Option<String>, #[serde(default)] access_token: Option<String>, #[serde(default)] refresh_token: Option<String>, #[serde(default)] account_id: Option<String>, #[serde(default)] expires_in: Option<u64> }
#[derive(serde::Deserialize)] struct OAuthDto { id_token: String, access_token: String, refresh_token: String, #[serde(default)] account_id: Option<String>, #[serde(default)] expires_in: Option<u64> }

pub struct CodexAuth<T, S, P, C> { transport: T, storage: S, pkce: P, security: C }
impl<T, S, P, C> CodexAuth<T,S,P,C> where T: AuthTransport, S: AuthStorage, P: PkceProvider, C: SecurityCheck {
 pub fn new(transport:T, storage:S, pkce:P, security:C)->Self { Self{transport,storage,pkce,security} }
 pub fn begin(&mut self)->AuthResult<(DevicePrompt, PkceState, String, u64)> {
  if !self.security.secure() { return Err(AuthError::new(AuthErrorKind::Security,"secure storage unavailable")); }
  let pkce=self.pkce.generate()?;
  let body=format!(r#"{{"client_id":"{}"}}"#,OPENAI_CODEX_CLIENT_ID);
  let raw=self.transport.post_json(DEVICE_USERCODE_URL,&body)?;
  let (dto,_)=serde_json_core::from_str::<UserCodeDto>(&raw).map_err(|_|AuthError::new(AuthErrorKind::InvalidResponse,"invalid device authorization response"))?;
  Ok((DevicePrompt{user_code:dto.user_code,verification_url:VERIFICATION_URL},pkce,dto.device_auth_id,dto.interval.unwrap_or(5)))
 }
 pub fn complete(&mut self, device_auth_id:&str, user_code:&str, pkce:&PkceState, interval_secs:u64)->AuthResult<TokenRecord> {
  let started=SystemTime::now();
  loop {
   if started.elapsed().unwrap_or_default()>Duration::from_secs(900){return Err(AuthError::new(AuthErrorKind::Expired,"device authorization expired"));}
   let body=format!(r#"{{"device_auth_id":"{}","user_code":"{}"}}"#,device_auth_id,user_code);
   match self.transport.post_json(DEVICE_TOKEN_URL,&body) {
    Ok(raw)=>{let (dto,_)=serde_json_core::from_str::<DeviceTokenDto>(&raw).map_err(|_|AuthError::new(AuthErrorKind::InvalidResponse,"invalid device token response"))?; let code=dto.authorization_code; let form=format!("grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",code,"http%3A%2F%2Flocalhost",OPENAI_CODEX_CLIENT_ID,pkce.verifier); let token_raw=self.transport.post_form(OAUTH_TOKEN_URL,&form)?; let (o,_)=serde_json_core::from_str::<OAuthDto>(&token_raw).map_err(|_|AuthError::new(AuthErrorKind::InvalidResponse,"invalid OAuth response"))?; let now=SystemTime::now(); let rec=TokenRecord{id_token:o.id_token,access_token:o.access_token,refresh_token:o.refresh_token,account_id:o.account_id,access_token_expires_at:now+Duration::from_secs(o.expires_in.unwrap_or(3600)),last_refresh_at:now}; self.storage.save(&rec)?; return Ok(rec);}
    Err(e) if e.kind==AuthErrorKind::Rejected=>std::thread::sleep(Duration::from_secs(interval_secs.max(1))),
    Err(e)=>return Err(e)
   }
  }
 }
 pub fn refresh_if_needed(&mut self, force:bool)->AuthResult<Option<TokenRecord>> { let old=match self.storage.load()? {Some(v)=>v,None=>return Ok(None)}; if !force && old.access_token_expires_at.duration_since(SystemTime::now()).unwrap_or_default()>Duration::from_secs(300) && old.last_refresh_at.elapsed().unwrap_or_default()<Duration::from_secs(8*86400){return Ok(Some(old));} let form=format!("grant_type=refresh_token&refresh_token={}&client_id={}",old.refresh_token,OPENAI_CODEX_CLIENT_ID); let raw=self.transport.post_form(OAUTH_TOKEN_URL,&form).map_err(|e|{if e.kind==AuthErrorKind::Rejected {AuthError::new(AuthErrorKind::Expired,"refresh rejected; reconnect required")} else {e}})?; let (o,_)=serde_json_core::from_str::<OAuthDto>(&raw).map_err(|_|AuthError::new(AuthErrorKind::InvalidResponse,"invalid refresh response"))?; let now=SystemTime::now(); let rec=TokenRecord{id_token:o.id_token,access_token:o.access_token,refresh_token:o.refresh_token,account_id:o.account_id,access_token_expires_at:now+Duration::from_secs(o.expires_in.unwrap_or(3600)),last_refresh_at:now}; self.storage.save(&rec)?; Ok(Some(rec)) }
 pub fn logout(&mut self)->AuthResult<()> { if let Some(t)=self.storage.load()? { let _=self.transport.post_form(OAUTH_REVOKE_URL,&format!("token={}",t.refresh_token)); } self.storage.erase() }
}
