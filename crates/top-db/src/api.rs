//! The JSON API behind the web page, shared by the desktop program (served
//! over HTTP) and the browser version (called directly from WebAssembly).
//!
//! `handle` takes a request (method, path under `/api/`, query string, body)
//! and returns a response (status, content type, body, file name for
//! downloads). It does no I/O of its own beyond the database.

use crate::export::{self, COLUMNS, ExportProfile, ExportSpec};
use crate::gpkg;
use crate::settings::Settings;
use crate::store::{Kind, PileState, Store};
use serde::Deserialize;
use serde_json::{Value, json};

pub struct Request<'a> {
    pub method: &'a str,
    /// Path, with or without the leading `/api/`: `summary`, `imports/3/void`...
    pub path: &'a str,
    /// Raw query string, without `?`.
    pub query: &'a str,
    pub body: &'a [u8],
}

pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    /// Set for downloads.
    pub filename: Option<String>,
}

/// What the program around the database provides.
pub struct Host {
    /// Optimiser threads (1 in a browser).
    pub threads: usize,
    /// "desktop" or "browser": the page adapts a few controls.
    pub mode: &'static str,
    /// Watch-folder status shown on the page (desktop).
    pub watch: Value,
    /// Check that a watch folder exists on this computer when it is saved.
    pub check_folder: bool,
}

/// True for requests that change the database (the browser saves after them).
pub fn is_write(method: &str) -> bool {
    !method.eq_ignore_ascii_case("GET")
}

pub fn handle(store: &mut Store, host: &Host, req: &Request) -> Response {
    match route(store, host, req) {
        Ok(r) => r,
        Err((status, message)) => json_response(status, &json!({ "error": message })),
    }
}

type Routed = Result<Response, (u16, String)>;

fn json_response(status: u16, v: &Value) -> Response {
    Response {
        status,
        content_type: "application/json".into(),
        body: serde_json::to_vec(v).expect("JSON serialises"),
        filename: None,
    }
}

fn ok(v: Value) -> Routed {
    Ok(json_response(200, &v))
}

fn bad(e: String) -> (u16, String) {
    (400, e)
}

fn download(name: String, content_type: &str, body: Vec<u8>) -> Routed {
    Ok(Response {
        status: 200,
        content_type: content_type.into(),
        body,
        filename: Some(name),
    })
}

const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H%M").to_string()
}

/// Value of a query parameter, percent-decoded (`+` is a space).
pub fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (decode(k) == key).then(|| decode(v))
    })
}

fn decode(s: &str) -> String {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h * 16 + l);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn r(v: f64, decimals: i32) -> f64 {
    let k = 10f64.powi(decimals);
    (v * k).round() / k
}

/// Piles as parallel arrays (compact for ~40,000 piles) plus the tracker list.
fn state_columns(store: &Store) -> Result<Value, String> {
    let state = store.state()?;
    let p = &state.piles;
    let col =
        |f: &dyn Fn(&PileState) -> Value| -> Value { Value::Array(p.iter().map(f).collect()) };
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
        "ttype": col(&|p| json!(p.tracker_type)),
        "flood": col(&|p| opt(p.flood, 3)),
        "flvl": col(&|p| opt(p.flood_level, 4)),
        "mintop": col(&|p| opt(p.min_top, 4)),
        "mta": col(&|p| json!(p.result.as_ref().is_some_and(|r| r.min_top_active))),
        "se": col(&|p| opt(p.result.as_ref().and_then(|r| r.set_easting), 4)),
        "sn": col(&|p| opt(p.result.as_ref().and_then(|r| r.set_northing), 4)),
        "shift": col(&|p| opt(p.result.as_ref().and_then(|r| r.shift), 4)),
    });
    Ok(json!({ "piles": piles, "trackers": state.trackers }))
}

#[derive(Deserialize)]
struct VoidBody {
    reason: String,
}

#[derive(Deserialize, Default)]
struct OptimizeBody {
    #[serde(default)]
    force: bool,
}

#[derive(Deserialize)]
struct SettingsBody {
    settings: Settings,
    watch_folder: Option<String>,
}

/// Custom export from the query: `spec` (JSON of `ExportSpec`) or `profile`
/// (a saved profile's columns and sheets), plus `blocks` (JSON list).
fn export_spec(store: &Store, query: &str) -> Result<Option<ExportSpec>, (u16, String)> {
    let mut spec = match (query_param(query, "spec"), query_param(query, "profile")) {
        (Some(json), _) => serde_json::from_str::<ExportSpec>(&json)
            .map_err(|e| bad(format!("invalid export choice: {e}")))?,
        (None, Some(name)) => {
            let p = store.export_profile(&name).map_err(bad)?;
            ExportSpec {
                columns: p.columns,
                blocks: Vec::new(),
                sheets: p.sheets,
            }
        }
        (None, None) => return Ok(None),
    };
    if let Some(blocks) = query_param(query, "blocks") {
        spec.blocks =
            serde_json::from_str(&blocks).map_err(|e| bad(format!("invalid block list: {e}")))?;
    }
    spec.validate().map_err(bad)?;
    Ok(Some(spec))
}

/// Part of a file name for a block choice.
fn blocks_label(spec: &ExportSpec) -> String {
    match spec.blocks.as_slice() {
        [] => String::new(),
        [b] => format!("_block-{}", crate::blocks::safe_name(b)),
        many => format!("_{}-blocks", many.len()),
    }
}

fn json_body<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, (u16, String)> {
    serde_json::from_slice(body).map_err(|e| bad(format!("invalid request: {e}")))
}

fn route(store: &mut Store, host: &Host, req: &Request) -> Routed {
    let path = req.path.trim_start_matches('/');
    let path = path.strip_prefix("api/").unwrap_or(path);
    let method = req.method.to_ascii_uppercase();
    let threads = host.threads;
    match (method.as_str(), path) {
        ("GET", "summary") => {
            let state = store.state().map_err(bad)?;
            ok(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "mode": host.mode,
                "summary": store.summary_of(&state).map_err(bad)?,
                "installers": store.block_status_of(&state).map_err(bad)?,
                "settings": store.settings().map_err(bad)?,
                "watch_folder": store.watch_folder().map_err(bad)?,
                "watch": host.watch,
            }))
        }
        ("GET", "state") => ok(state_columns(store).map_err(bad)?),
        ("GET", "imports") => ok(json!(store.imports().map_err(bad)?)),
        ("GET", p) if p.starts_with("imports/") => {
            let id: i64 = p["imports/".len()..]
                .parse()
                .map_err(|_| (404, "not found".to_string()))?;
            match store.import_detail(id).map_err(bad)? {
                Some(row) => ok(json!(row)),
                None => Err((404, format!("import #{id} does not exist"))),
            }
        }
        ("POST", p) if p.starts_with("imports/") && p.ends_with("/void") => {
            let id: i64 = p["imports/".len()..p.len() - "/void".len()]
                .parse()
                .map_err(|_| (404, "not found".to_string()))?;
            let body: VoidBody = json_body(req.body)?;
            store.void_import(id, &body.reason).map_err(bad)?;
            let run = store
                .optimize(false, &format!("import #{id} voided"), threads)
                .map_err(bad)?;
            ok(json!({ "run": run }))
        }
        ("POST", "import") => {
            let kind_s = query_param(req.query, "kind").unwrap_or_default();
            let kind = Kind::parse(&kind_s)
                .ok_or_else(|| bad(format!("unknown import kind \"{kind_s}\"")))?;
            let name = query_param(req.query, "name").unwrap_or_else(|| "file".into());
            let date = query_param(req.query, "date").filter(|d| !d.trim().is_empty());
            let origin = query_param(req.query, "origin").unwrap_or_else(|| "upload".into());
            let report = store
                .import_file(kind, &name, req.body, date.as_deref(), &origin)
                .map_err(bad)?;
            let run = if report.accepted {
                let trigger = format!(
                    "{} import #{}",
                    kind.as_str(),
                    report.import_id.unwrap_or(0)
                );
                Some(store.optimize(false, &trigger, threads).map_err(bad)?)
            } else {
                None
            };
            ok(json!({ "report": report, "run": run }))
        }
        ("GET", "file_seen") => {
            let kind = Kind::parse(&query_param(req.query, "kind").unwrap_or_default())
                .ok_or_else(|| bad("unknown kind".into()))?;
            let sha = query_param(req.query, "sha").unwrap_or_default();
            ok(json!({ "seen": store.file_seen(kind, &sha).map_err(bad)? }))
        }
        ("POST", "optimize") => {
            let body: OptimizeBody = if req.body.is_empty() {
                OptimizeBody::default()
            } else {
                json_body(req.body)?
            };
            let trigger = if body.force {
                "manual (all trackers)"
            } else {
                "manual"
            };
            ok(json!(
                store.optimize(body.force, trigger, threads).map_err(bad)?
            ))
        }
        ("GET", "settings") => ok(json!({
            "settings": store.settings().map_err(bad)?,
            "defaults": Settings::default(),
            "watch_folder": store.watch_folder().map_err(bad)?,
        })),
        ("PUT", "settings") => {
            let body: SettingsBody = json_body(req.body)?;
            if host.check_folder
                && let Some(folder) = body
                    .watch_folder
                    .as_deref()
                    .map(str::trim)
                    .filter(|f| !f.is_empty())
                && !std::path::Path::new(folder).is_dir()
            {
                return Err(bad(format!(
                    "the watch folder \"{folder}\" does not exist on this computer"
                )));
            }
            store.set_settings(&body.settings).map_err(bad)?;
            store
                .set_watch_folder(body.watch_folder.as_deref())
                .map_err(bad)?;
            let run = store
                .optimize(false, "settings changed", threads)
                .map_err(bad)?;
            ok(json!({ "run": run }))
        }
        ("GET", "export.xlsx") => {
            let spec = export_spec(store, req.query)?.unwrap_or_default();
            let bytes = export::workbook_with(
                &store.state().map_err(bad)?,
                &store.imports().map_err(bad)?,
                &store.settings().map_err(bad)?,
                &spec,
                &[],
            )
            .map_err(bad)?;
            download(
                format!("tracker-top{}_{}.xlsx", blocks_label(&spec), stamp()),
                XLSX,
                bytes,
            )
        }
        ("GET", "export.csv") => {
            let spec = export_spec(store, req.query)?.unwrap_or_default();
            let text = export::piles_csv_with(&store.state().map_err(bad)?, &spec).map_err(bad)?;
            download(
                format!("tracker-top_piles{}_{}.csv", blocks_label(&spec), stamp()),
                "text/csv; charset=utf-8",
                text.into_bytes(),
            )
        }
        ("GET", "export/options") => {
            let summary = store.summary().map_err(bad)?;
            ok(json!({
                "columns": COLUMNS,
                "sheets": export::SHEETS,
                "profiles": store.export_profiles().map_err(bad)?,
                "blocks": summary.blocks,
            }))
        }
        ("PUT", "export/profiles") => {
            let profiles: Vec<ExportProfile> = json_body(req.body)?;
            store.set_export_profiles(&profiles).map_err(bad)?;
            ok(json!({ "profiles": store.export_profiles().map_err(bad)? }))
        }
        ("GET", "blocks") => ok(json!(store.block_status().map_err(bad)?)),
        ("GET", "export/block.xlsx") => {
            let block = query_param(req.query, "block").unwrap_or_default();
            let (name, bytes) = store.block_workbook(&block).map_err(bad)?;
            download(name, XLSX, bytes)
        }
        ("POST", "export/block_done") => {
            let p = |k| query_param(req.query, k).unwrap_or_default();
            let rev = store
                .record_block_export(&p("block"), &p("sha"), &p("file"), &p("by"))
                .map_err(bad)?;
            ok(json!({ "revision": rev }))
        }
        ("GET", "sync/status") => ok(json!(store.sync_status().map_err(bad)?)),
        ("POST", "sync/incoming") => {
            let by = query_param(req.query, "by").unwrap_or_default();
            ok(json!(store.incoming(req.body, &by, threads).map_err(bad)?))
        }
        ("POST", "sync/adopt") => {
            store.adopt(req.body, threads).map_err(bad)?;
            ok(json!({ "status": store.sync_status().map_err(bad)? }))
        }
        ("POST", "sync/snapshot") => {
            let by = query_param(req.query, "by").unwrap_or_default();
            let bytes = store.snapshot(&by).map_err(bad)?;
            download("tracker-top.db".into(), "application/vnd.sqlite3", bytes)
        }
        ("POST", "sync/saved") => {
            store.mark_saved().map_err(bad)?;
            ok(json!({ "status": store.sync_status().map_err(bad)? }))
        }
        ("GET", "export.gpkg") => {
            let bytes = gpkg::geopackage(
                &store.state().map_err(bad)?,
                &store.settings().map_err(bad)?,
            )
            .map_err(bad)?;
            download(
                format!("tracker-top_{}.gpkg", stamp()),
                "application/geopackage+sqlite3",
                bytes,
            )
        }
        ("GET", "backup.db") => {
            let bytes = store.to_bytes().map_err(bad)?;
            download(
                format!("tracker-top_{}.db", stamp()),
                "application/vnd.sqlite3",
                bytes,
            )
        }
        _ => Err((404, format!("no such request: {method} /api/{path}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../testdata/sample_block01_synthetic.csv"
    ));

    fn host() -> Host {
        Host {
            threads: 1,
            mode: "browser",
            watch: Value::Null,
            check_folder: false,
        }
    }

    fn call(s: &mut Store, method: &str, path: &str, query: &str, body: &[u8]) -> (u16, Value) {
        let r = handle(
            s,
            &host(),
            &Request {
                method,
                path,
                query,
                body,
            },
        );
        let v = if r.content_type == "application/json" {
            serde_json::from_slice(&r.body).unwrap()
        } else {
            json!({ "bytes": r.body.len(), "filename": r.filename })
        };
        (r.status, v)
    }

    #[test]
    fn query_decoding() {
        let q = "kind=survey&name=Leve%20pieux+03.11.2026.csv&date=";
        assert_eq!(
            query_param(q, "name").as_deref(),
            Some("Leve pieux 03.11.2026.csv")
        );
        assert_eq!(query_param(q, "date").as_deref(), Some(""));
        assert_eq!(query_param(q, "missing"), None);
        assert_eq!(query_param("a=%C3%A9%2", "a").as_deref(), Some("é%2"));
    }

    #[test]
    fn requests_end_to_end_and_backup_round_trip() {
        let mut s = Store::open_in_memory().unwrap();
        let (st, v) = call(
            &mut s,
            "POST",
            "/api/import",
            "kind=master&name=piles.csv",
            SAMPLE,
        );
        assert_eq!(st, 200, "{v}");
        assert_eq!(v["report"]["accepted"], true);
        assert_eq!(v["run"]["trackers_run"], 62);
        let (_, v) = call(&mut s, "GET", "api/summary", "", b"");
        assert_eq!(
            (v["summary"]["piles"].as_u64(), v["mode"].as_str()),
            (Some(789), Some("browser"))
        );
        let (_, v) = call(&mut s, "GET", "state", "", b"");
        assert_eq!(v["piles"]["id"].as_array().unwrap().len(), 789);
        let (st, v) = call(&mut s, "GET", "export.gpkg", "", b"");
        assert_eq!(st, 200);
        assert!(v["filename"].as_str().unwrap().ends_with(".gpkg"));
        let (st, _) = call(&mut s, "GET", "nope", "", b"");
        assert_eq!(st, 404);
        let (st, v) = call(&mut s, "POST", "imports/99/void", "", br#"{"reason":"x"}"#);
        assert_eq!(st, 400, "{v}");

        // Backup, then reopen from the bytes: everything is there.
        let backup = handle(
            &mut s,
            &host(),
            &Request {
                method: "GET",
                path: "backup.db",
                query: "",
                body: b"",
            },
        );
        let mut copy = Store::from_bytes(&backup.body).unwrap();
        assert_eq!(copy.pile_count().unwrap(), 789);
        let (_, v) = call(&mut copy, "GET", "summary", "", b"");
        assert_eq!(
            v["summary"]["tracker_status"]["Compliant"]
                .as_u64()
                .unwrap()
                + v["summary"]["tracker_status"]["Graded"]
                    .as_u64()
                    .unwrap_or(0),
            62
        );
        assert!(Store::from_bytes(b"not a database").is_err());
        let gpkg = handle(
            &mut s,
            &host(),
            &Request {
                method: "GET",
                path: "export.gpkg",
                query: "",
                body: b"",
            },
        );
        assert!(
            Store::from_bytes(&gpkg.body).is_err(),
            "a GeoPackage is not a tracker-top database"
        );
    }
}
