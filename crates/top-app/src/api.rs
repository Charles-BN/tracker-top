//! HTTP routes: the JSON API used by the web page, file exports and the page itself.

use crate::assets;
use crate::watch::WatchStatus;
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use serde_json::json;
use std::sync::{Arc, Mutex};
use top_db::Store;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub threads: usize,
    pub watch: Arc<Mutex<WatchStatus>>,
}

pub struct ApiError(StatusCode, String);

impl ApiError {
    pub fn message(&self) -> &str {
        &self.1
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

/// Run database work on a blocking thread, one request at a time.
pub async fn with_store<T, F>(store: &Arc<Mutex<Store>>, f: F) -> ApiResult<T>
where
    T: Send + 'static,
    F: FnOnce(&mut Store) -> Result<T, String> + Send + 'static,
{
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut s = store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut s)
    })
    .await
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/{*rest}", any(api))
        .fallback(static_file)
        .layer(DefaultBodyLimit::max(512 * 1024 * 1024))
        .with_state(state)
}

async fn static_file(uri: Uri) -> Response {
    match assets::find(uri.path()) {
        Some(a) => (
            [
                (header::CONTENT_TYPE, a.mime),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            a.body,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// Every `/api/...` request goes to the shared handler in `top_db::api`,
/// the same one the browser version calls.
async fn api(State(st): State<AppState>, method: Method, uri: Uri, body: Bytes) -> Response {
    let watch = st.watch.lock().map(|w| w.clone()).unwrap_or_default();
    let host = top_db::api::Host {
        threads: st.threads,
        mode: "desktop",
        watch: serde_json::to_value(watch).unwrap_or_default(),
        check_folder: true,
    };
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("").to_string();
    let m = method.as_str().to_string();
    let store = st.store.clone();
    let res = tokio::task::spawn_blocking(move || {
        let mut s = store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        top_db::api::handle(
            &mut s,
            &host,
            &top_db::api::Request {
                method: &m,
                path: &path,
                query: &query,
                body: &body,
            },
        )
    })
    .await;
    let res = match res {
        Ok(r) => r,
        Err(e) => {
            return ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    if method == Method::PUT
        && uri.path().ends_with("/settings")
        && res.status == 200
        && let Ok(mut w) = st.watch.lock()
    {
        w.rescan_now = true;
    }
    let mut resp = (
        StatusCode::from_u16(res.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        [(header::CONTENT_TYPE, res.content_type)],
        res.body,
    )
        .into_response();
    if let Some(name) = res.filename
        && let Ok(v) = content_disposition(&name).parse()
    {
        resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    resp
}

/// `attachment` header for a file name, any characters (RFC 6266 / 5987).
fn content_disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}
