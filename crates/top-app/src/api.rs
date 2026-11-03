//! HTTP routes: the JSON API used by the web page, file exports and the page itself.

use crate::assets;
use crate::watch::WatchStatus;
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use top_db::{Kind, Settings, Store, export};

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
        .route("/api/summary", get(summary))
        .route("/api/state", get(state_columns))
        .route("/api/imports", get(imports))
        .route("/api/imports/{id}", get(import_detail))
        .route("/api/imports/{id}/void", post(void_import))
        .route("/api/import", post(import))
        .route("/api/optimize", post(optimize))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/export.xlsx", get(export_xlsx))
        .route("/api/export.csv", get(export_csv))
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

async fn summary(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let watch = st.watch.lock().map(|w| w.clone()).unwrap_or_default();
    let (summary, settings, folder) = with_store(&st.store, |s| {
        Ok((s.summary()?, s.settings()?, s.watch_folder()?))
    })
    .await?;
    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "summary": summary,
        "settings": settings,
        "watch_folder": folder,
        "watch": watch,
    })))
}

fn r(v: f64, decimals: i32) -> f64 {
    let k = 10f64.powi(decimals);
    (v * k).round() / k
}

/// Piles as parallel arrays (compact for ~40,000 piles) plus the tracker list.
async fn state_columns(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let state = with_store(&st.store, |s| s.state()).await?;
    let p = &state.piles;
    let col = |f: &dyn Fn(&top_db::PileState) -> Value| -> Value {
        Value::Array(p.iter().map(f).collect())
    };
    let opt = |v: Option<f64>, d: i32| v.map(|v| json!(r(v, d))).unwrap_or(Value::Null);
    let piles = json!({
        "id": col(&|p| json!(p.pile_id)),
        "tracker": col(&|p| json!(p.tracker)),
        "block": col(&|p| json!(p.block)),
        "section": col(&|p| json!(p.section)),
        "e": col(&|p| json!(r(p.easting, 4))),
        "n": col(&|p| json!(r(p.northing, 4))),
        "type": col(&|p| json!(p.pile_type)),
        "color": col(&|p| json!(p.color)),
        "z": col(&|p| opt(p.z, 4)),
        "zsrc": col(&|p| json!(p.z_source.map(Kind::as_str))),
        "zdate": col(&|p| json!(p.z_date)),
        "offset": col(&|p| opt(p.offset_m, 3)),
        "nsurv": col(&|p| json!(p.n_surveys)),
        "run": col(&|p| json!(p.result.as_ref().map(|r| r.run_id))),
        "zrun": col(&|p| opt(p.result.as_ref().map(|r| r.z_ground), 4)),
        "ng": col(&|p| opt(p.result.as_ref().map(|r| r.new_ground), 4)),
        "cf": col(&|p| opt(p.result.as_ref().map(|r| r.cut_fill), 4)),
        "top": col(&|p| opt(p.result.as_ref().map(|r| r.top), 4)),
        "rev": col(&|p| opt(p.result.as_ref().map(|r| r.reveal), 4)),
        "ang": col(&|p| opt(p.result.as_ref().map(|r| r.angle_diff_deg), 4)),
        "outl": col(&|p| json!(p.result.as_ref().is_some_and(|r| r.outlier))),
    });
    Ok(Json(json!({ "piles": piles, "trackers": state.trackers })))
}

async fn imports(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let list = with_store(&st.store, |s| s.imports()).await?;
    Ok(Json(json!(list)))
}

async fn import_detail(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    match with_store(&st.store, move |s| s.import_detail(id)).await? {
        Some(row) => Ok(Json(json!(row))),
        None => Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("import #{id} does not exist"),
        )),
    }
}

#[derive(Deserialize)]
struct VoidBody {
    reason: String,
}

async fn void_import(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<VoidBody>,
) -> ApiResult<Json<Value>> {
    let threads = st.threads;
    let run = with_store(&st.store, move |s| {
        s.void_import(id, &body.reason)?;
        s.optimize(false, &format!("import #{id} voided"), threads)
    })
    .await?;
    Ok(Json(json!({ "run": run })))
}

#[derive(Deserialize)]
struct ImportQuery {
    kind: String,
    name: String,
    date: Option<String>,
}

async fn import(
    State(st): State<AppState>,
    Query(q): Query<ImportQuery>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let kind = Kind::parse(&q.kind).ok_or_else(|| {
        ApiError(
            StatusCode::BAD_REQUEST,
            format!("unknown import kind \"{}\"", q.kind),
        )
    })?;
    let threads = st.threads;
    let (report, run) = with_store(&st.store, move |s| {
        let date = q.date.as_deref().filter(|d| !d.trim().is_empty());
        let report = s.import_file(kind, &q.name, &body, date, "upload")?;
        let run = if report.accepted {
            Some(s.optimize(
                false,
                &format!(
                    "{} import #{}",
                    kind.as_str(),
                    report.import_id.unwrap_or(0)
                ),
                threads,
            )?)
        } else {
            None
        };
        Ok((report, run))
    })
    .await?;
    Ok(Json(json!({ "report": report, "run": run })))
}

#[derive(Deserialize, Default)]
struct OptimizeBody {
    #[serde(default)]
    force: bool,
}

async fn optimize(
    State(st): State<AppState>,
    body: Option<Json<OptimizeBody>>,
) -> ApiResult<Json<Value>> {
    let force = body.map(|b| b.force).unwrap_or(false);
    let threads = st.threads;
    let run = with_store(&st.store, move |s| {
        s.optimize(
            force,
            if force {
                "manual (all trackers)"
            } else {
                "manual"
            },
            threads,
        )
    })
    .await?;
    Ok(Json(json!(run)))
}

async fn get_settings(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let (settings, folder) =
        with_store(&st.store, |s| Ok((s.settings()?, s.watch_folder()?))).await?;
    Ok(Json(
        json!({ "settings": settings, "defaults": Settings::default(), "watch_folder": folder }),
    ))
}

#[derive(Deserialize)]
struct SettingsBody {
    settings: Settings,
    watch_folder: Option<String>,
}

async fn put_settings(
    State(st): State<AppState>,
    Json(body): Json<SettingsBody>,
) -> ApiResult<Json<Value>> {
    if let Some(folder) = body
        .watch_folder
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
        && !std::path::Path::new(folder).is_dir()
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            format!("the watch folder \"{folder}\" does not exist on this computer"),
        ));
    }
    let threads = st.threads;
    let run = with_store(&st.store, move |s| {
        s.set_settings(&body.settings)?;
        s.set_watch_folder(body.watch_folder.as_deref())?;
        s.optimize(false, "settings changed", threads)
    })
    .await?;
    if let Ok(mut w) = st.watch.lock() {
        w.rescan_now = true;
    }
    Ok(Json(json!({ "run": run })))
}

fn attachment(name: &str, mime: &'static str, body: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        body,
    )
        .into_response()
}

fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H%M").to_string()
}

async fn export_xlsx(State(st): State<AppState>) -> ApiResult<Response> {
    let bytes = with_store(&st.store, |s| {
        export::workbook(&s.state()?, &s.imports()?, &s.settings()?)
    })
    .await?;
    Ok(attachment(
        &format!("tracker-top_{}.xlsx", stamp()),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        bytes,
    ))
}

async fn export_csv(State(st): State<AppState>) -> ApiResult<Response> {
    let text = with_store(&st.store, |s| Ok(export::piles_csv(&s.state()?))).await?;
    Ok(attachment(
        &format!("tracker-top_piles_{}.csv", stamp()),
        "text/csv; charset=utf-8",
        text.into_bytes(),
    ))
}
