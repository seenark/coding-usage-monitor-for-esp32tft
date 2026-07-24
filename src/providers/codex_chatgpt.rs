//! Experimental Codex quota adapter. Provider wire details stay isolated here.
use std::time::{Duration, Instant, SystemTime};
use std::string::String;
use heapless::String as HeaplessString;
use crate::domain::{Connectivity, Credits, UsageError, UsageProvider, UsageState, WindowUsage};

pub const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
pub const USER_AGENT: &str = "codex-usage-monitor/0.1 experimental";

pub trait UsageTransport { fn get(&mut self, url:&str, headers:&[(&str,&str)]) -> Result<(u16,String), UsageError>; }
pub trait TokenSource { fn access_token(&mut self, force_refresh:bool) -> Result<TokenView, UsageError>; }
pub struct TokenView { pub access_token:String, pub account_id:Option<String> }

#[derive(serde::Deserialize)] struct UsageDto { plan_type:String, rate_limit:RateLimit, #[serde(default)] credits:Option<CreditsDto> }
#[derive(serde::Deserialize)] struct RateLimit { primary_window:WindowDto, secondary_window:WindowDto }
#[derive(serde::Deserialize)] struct WindowDto { used_percent:u8, #[serde(default)] reset_after_seconds:Option<u64>, #[serde(default)] reset_at:Option<u64> }
#[derive(serde::Deserialize)] struct CreditsDto { balance:String }

pub struct CodexUsageClient<T, A> { transport:T, auth:A, last_valid:Option<UsageState>, now:fn()->SystemTime }
impl<T,A> CodexUsageClient<T,A> where T:UsageTransport, A:TokenSource {
 pub fn new(transport:T, auth:A)->Self { Self{transport,auth,last_valid:None,now:SystemTime::now} }
 pub fn last_valid(&self)->Option<&UsageState>{self.last_valid.as_ref()}
 fn request(&mut self, force:bool)->Result<UsageState,UsageError> {
  let tok=self.auth.access_token(force)?; let mut headers:Vec<(&str,&str)>=Vec::with_capacity(4); let auth=format!("Bearer {}",tok.access_token); headers.push(("Authorization",auth.as_str())); if let Some(id)=&tok.account_id {headers.push(("chatgpt-account-id",id.as_str()));} headers.push(("Accept","application/json")); headers.push(("User-Agent",USER_AGENT)); let (status,body)=self.transport.get(USAGE_URL,&headers)?;
  if status==401 || status==403 { return Err(UsageError::Unauthorized); } if !(200..300).contains(&status){return Err(UsageError::Transport);}
  let (dto,_)=serde_json_core::from_str::<UsageDto>(&body).map_err(|_|UsageError::InvalidResponse)?; let now=(self.now)(); let reset=|w:&WindowDto| -> SystemTime { if let Some(s)=w.reset_after_seconds {now+Duration::from_secs(s)} else if let Some(ts)=w.reset_at {SystemTime::UNIX_EPOCH+Duration::from_secs(ts)} else {now} }; let mut plan=HeaplessString::<16>::new(); plan.push_str(&dto.plan_type).map_err(|_|UsageError::InvalidResponse)?; let credits=dto.credits.map(|c|{let mut balance=HeaplessString::<24>::new(); let _=balance.push_str(&c.balance); Credits{balance}}); Ok(UsageState{plan,five_hour:WindowUsage{used_percent:dto.rate_limit.primary_window.used_percent.min(100),resets_at:reset(&dto.rate_limit.primary_window)},weekly:WindowUsage{used_percent:dto.rate_limit.secondary_window.used_percent.min(100),resets_at:reset(&dto.rate_limit.secondary_window)},credits,connectivity:Connectivity::Online,last_updated:Instant::now()})
 }
}
impl<T,A> UsageProvider for CodexUsageClient<T,A> where T:UsageTransport,A:TokenSource {
 fn refresh(&mut self)->Result<UsageState,UsageError>{ match self.request(false) { Ok(v)=>{self.last_valid=Some(v.clone());Ok(v)}, Err(UsageError::Unauthorized)=>{match self.request(true){Ok(v)=>{self.last_valid=Some(v.clone());Ok(v)},Err(e)=>self.last_valid.clone().ok_or(e)}}, Err(e)=>self.last_valid.clone().ok_or(e) } }
}
