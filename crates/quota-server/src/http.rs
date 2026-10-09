//! Admin app backend: JSON API under `/api`, static files and SPA fallback for everything else.
//! No CORS (same origin). No route except `/api/device-setup` returns a secret.
use crate::{
    config::DeviceSetupConfig,
    mqtt::Publisher,
    scheduler::Control,
    settings::{DeviceSettings, FieldErrors, ServerSettings, Store},
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, FromRequest, Path, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use quota_core::Provider;
use serde_json::{Value, json};
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::{net::TcpListener, sync::watch};
use tower_http::services::{ServeDir, ServeFile};

pub const BODY_LIMIT: usize = 16 * 1024;
const ASSET_CACHE: &str = "public, max-age=31536000, immutable";

pub struct AppState {
    pub mock: bool,
    pub store: Store,
    pub settings: watch::Sender<ServerSettings>,
    pub device: Mutex<DeviceSettings>,
    /// Serializes "write file, then apply" so concurrent PUTs cannot interleave.
    pub write_lock: tokio::sync::Mutex<()>,
    pub claude: Arc<Control>,
    pub codex: Arc<Control>,
    pub publisher: Publisher,
    pub device_setup: Option<DeviceSetupConfig>,
    pub web_dist_dir: PathBuf,
}
impl AppState {
    fn control(&self, provider: Provider) -> &Control {
        match provider {
            Provider::Claude => &self.claude,
            Provider::Codex => &self.codex,
        }
    }
}

struct ApiError(StatusCode, Value);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}
fn error(status: StatusCode, code: &str) -> ApiError {
    ApiError(status, json!({"error": code}))
}
fn invalid(fields: FieldErrors) -> ApiError {
    ApiError(
        StatusCode::BAD_REQUEST,
        json!({"error": "invalid", "fields": fields}),
    )
}
fn storage(_: io::Error) -> ApiError {
    error(StatusCode::INTERNAL_SERVER_ERROR, "storage")
}

/// Reads at most `BODY_LIMIT` bytes of JSON; failures use the common `{"error":...}` shape.
struct JsonBody(Value);
impl<S: Send + Sync> FromRequest<S> for JsonBody {
    type Rejection = ApiError;
    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|rejection| {
                if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    error(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large")
                } else {
                    error(StatusCode::BAD_REQUEST, "invalid_body")
                }
            })?;
        serde_json::from_slice(&bytes)
            .map(Self)
            .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_json"))
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let index = state.web_dist_dir.join("index.html");
    let files = ServeDir::new(&state.web_dist_dir).fallback(ServeFile::new(index));
    let api = Router::new()
        .route("/status", get(status))
        .route("/settings", get(get_settings).put(put_settings))
        .route("/refresh/{provider}", post(refresh))
        .route("/device", get(get_device).put(put_device))
        .route("/device-setup", get(device_setup))
        .fallback(|| async { error(StatusCode::NOT_FOUND, "not_found") })
        .method_not_allowed_fallback(|| async {
            error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed")
        })
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state);
    Router::new()
        .nest("/api", api)
        // `nest` leaves the trailing-slash root to the SPA fallback; keep `/api/` JSON.
        .route(
            "/api/",
            any(|| async { error(StatusCode::NOT_FOUND, "not_found") }),
        )
        .fallback_service(files)
        .layer(middleware::from_fn(headers))
}

pub async fn serve(
    listener: TcpListener,
    state: Arc<AppState>,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async move {
            let _ = shutdown.changed().await;
        })
        .await
}

fn is_json(request: &Request) -> bool {
    request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            let media = value.split(';').next().unwrap_or("").trim();
            media.eq_ignore_ascii_case("application/json")
        })
}
fn has_body(request: &Request) -> bool {
    request
        .headers()
        .get(header::CONTENT_LENGTH)
        .is_some_and(|value| value != "0")
        || request.headers().contains_key(header::TRANSFER_ENCODING)
}

/// `/api`: Content-Type gate for bodies and `no-store`. Assets: immutable. The rest: revalidate.
async fn headers(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let api = path == "/api" || path.starts_with("/api/");
    let writes = matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH
    );
    let mut response = if api && writes && has_body(&request) && !is_json(&request) {
        error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type").into_response()
    } else {
        next.run(request).await
    };
    let html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"));
    let cache = if api {
        "no-store"
    } else if path.starts_with("/assets/") && response.status().is_success() && !html {
        ASSET_CACHE
    } else {
        "no-cache"
    };
    if api || !response.headers().contains_key(header::CACHE_CONTROL) {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    }
    response
}

async fn status(State(state): State<Arc<AppState>>) -> Json<Value> {
    let settings = state.settings.borrow().clone();
    let provider = |provider: Provider| {
        let enabled = settings.provider(provider).enabled;
        let control = state.control(provider);
        json!({
            "enabled": enabled,
            "snapshot": control.snapshot(),
            "next_refresh_at": enabled.then(|| control.next_refresh_at()).flatten(),
            "retry_after_seconds": enabled.then(|| control.retry_after_seconds()).flatten(),
        })
    };
    Json(json!({
        "server": {"mock": state.mock},
        "device": {"state": state.publisher.device_state().as_str()},
        "providers": {
            "claude": provider(Provider::Claude),
            "codex": provider(Provider::Codex),
        },
    }))
}

async fn get_settings(State(state): State<Arc<AppState>>) -> Json<ServerSettings> {
    Json(state.settings.borrow().clone())
}

async fn put_settings(
    State(state): State<Arc<AppState>>,
    JsonBody(body): JsonBody,
) -> Result<Json<ServerSettings>, ApiError> {
    let settings = ServerSettings::from_json(&body).map_err(invalid)?;
    let _guard = state.write_lock.lock().await;
    state
        .store
        .save_settings(&settings)
        .await
        .map_err(storage)?;
    // Wakes every scheduler; they re-read the live values.
    state.settings.send_replace(settings.clone());
    Ok(Json(settings))
}

async fn refresh(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let provider = match name.as_str() {
        "claude" => Provider::Claude,
        "codex" => Provider::Codex,
        _ => return Err(error(StatusCode::NOT_FOUND, "not_found")),
    };
    if !state.settings.borrow().provider(provider).enabled {
        return Err(error(StatusCode::CONFLICT, "disabled"));
    }
    state
        .control(provider)
        .request_refresh()
        .map_err(|seconds| {
            ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                json!({"error": "rate_limited", "retry_after_seconds": seconds}),
            )
        })?;
    Ok((StatusCode::ACCEPTED, Json(json!({"scheduled": true}))))
}

async fn get_device(State(state): State<Arc<AppState>>) -> Json<DeviceSettings> {
    Json(
        state
            .device
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    )
}

async fn put_device(
    State(state): State<Arc<AppState>>,
    JsonBody(body): JsonBody,
) -> Result<Json<DeviceSettings>, ApiError> {
    let device = DeviceSettings::from_json(&body).map_err(invalid)?;
    let _guard = state.write_lock.lock().await;
    state.store.save_device(&device).await.map_err(storage)?;
    *state.device.lock().unwrap_or_else(|e| e.into_inner()) = device.clone();
    if state
        .publisher
        .publish_device_config(device.mqtt_payload())
        .is_err()
    {
        tracing::warn!("Device config queued for MQTT reconnect");
    }
    Ok(Json(device))
}

async fn device_setup(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let setup = state
        .device_setup
        .as_ref()
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "not_configured"))?;
    let unavailable = || error(StatusCode::SERVICE_UNAVAILABLE, "unavailable");
    let path = setup.password_file.as_ref().ok_or_else(unavailable)?;
    let password = tokio::fs::read_to_string(path)
        .await
        .map_err(|_| unavailable())?;
    let password = password.trim();
    if password.is_empty() {
        return Err(unavailable());
    }
    Ok(Json(json!({
        "host": setup.host,
        "port": setup.port,
        "username": "esp32",
        "password": password,
        "tls": false,
    })))
}
