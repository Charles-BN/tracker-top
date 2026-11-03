//! Watch folder: imports every new survey file that appears in a folder.
//!
//! Point it at the local copy of a Trimble Connect project folder kept up to
//! date by Trimble Connect Sync (or any synced or shared folder). Each scan
//! imports files it has not seen before as field surveys, then re-optimises
//! the trackers they touch. Files still being written (changed in the last
//! few seconds) wait for the next scan.

use crate::api::{AppState, with_store};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use top_db::{Kind, sha256_hex};

const EXTENSIONS: [&str; 7] = ["csv", "txt", "xlsx", "xlsm", "xls", "ods", "prn"];
const MAX_DEPTH: usize = 4;
const SETTLE: Duration = Duration::from_secs(5);
const MAX_EVENTS: usize = 50;

#[derive(Clone, Debug, Serialize)]
pub struct WatchEvent {
    pub at: String,
    pub file: String,
    pub accepted: bool,
    pub import_id: Option<i64>,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct WatchStatus {
    pub folder: Option<String>,
    pub last_scan: Option<String>,
    pub error: Option<String>,
    pub files_seen: usize,
    pub events: VecDeque<WatchEvent>,
    #[serde(skip)]
    pub rescan_now: bool,
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name.starts_with("~$") {
            continue;
        }
        let ft = entry.file_type()?;
        if ft.is_dir() && depth < MAX_DEPTH {
            collect(&path, depth + 1, out)?;
        } else if ft.is_file() {
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if EXTENSIONS.contains(&ext.as_str()) {
                out.push(path);
            }
        }
    }
    Ok(())
}

pub async fn run(state: AppState, interval_s: u64) {
    // path -> (size, modified) of files already handled.
    let mut handled: HashMap<PathBuf, (u64, SystemTime)> = HashMap::new();
    let mut current_folder: Option<String> = None;
    loop {
        let folder = with_store(&state.store, |s| s.watch_folder())
            .await
            .ok()
            .flatten();
        if folder != current_folder {
            handled.clear();
            current_folder = folder.clone();
        }
        if let Ok(mut w) = state.watch.lock() {
            w.folder = folder.clone();
            w.rescan_now = false;
        }
        if let Some(folder) = folder {
            scan(&state, Path::new(&folder), &mut handled).await;
        }
        // Sleep, waking early when the settings ask for a rescan.
        for _ in 0..interval_s {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if state.watch.lock().map(|w| w.rescan_now).unwrap_or(false) {
                break;
            }
        }
    }
}

async fn scan(state: &AppState, folder: &Path, handled: &mut HashMap<PathBuf, (u64, SystemTime)>) {
    let folder_buf = folder.to_path_buf();
    let listed = tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        collect(&folder_buf, 0, &mut files).map(|_| files)
    })
    .await;
    let files = match listed {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => {
            set_error(state, Some(format!("cannot read the folder: {e}")));
            return;
        }
        Err(e) => {
            set_error(state, Some(e.to_string()));
            return;
        }
    };
    set_error(state, None);

    let mut imported_any = false;
    let mut files = files;
    files.sort();
    for path in files {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let sig = (meta.len(), modified);
        if handled.get(&path) == Some(&sig) {
            continue;
        }
        if SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default()
            < SETTLE
        {
            continue; // still being written or synced
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        handled.insert(path.clone(), sig);
        let sha = sha256_hex(&bytes);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let result = with_store(&state.store, move |s| {
            if s.file_seen(Kind::Survey, &sha)? {
                return Ok(None);
            }
            s.import_file(Kind::Survey, &name, &bytes, None, "watch folder")
                .map(Some)
        })
        .await;
        let display = path
            .strip_prefix(folder)
            .unwrap_or(&path)
            .display()
            .to_string();
        let event = match result {
            Ok(None) => None,
            Ok(Some(rep)) => {
                imported_any |= rep.accepted;
                let message = if rep.accepted {
                    format!(
                        "{} piles updated ({} rows skipped), survey date {}",
                        rep.rows_used,
                        rep.rows_rejected,
                        rep.survey_date.clone().unwrap_or_default()
                    )
                } else {
                    rep.findings
                        .iter()
                        .find(|f| f.level == top_db::Level::Error)
                        .map(|f| f.message.clone())
                        .unwrap_or_else(|| "not imported".into())
                };
                Some(WatchEvent {
                    at: now(),
                    file: display,
                    accepted: rep.accepted,
                    import_id: rep.import_id,
                    message,
                })
            }
            Err(e) => Some(WatchEvent {
                at: now(),
                file: display,
                accepted: false,
                import_id: None,
                message: e.message().to_string(),
            }),
        };
        if let Some(ev) = event {
            println!("watch folder: {} - {}", ev.file, ev.message);
            if let Ok(mut w) = state.watch.lock() {
                w.events.push_front(ev);
                w.events.truncate(MAX_EVENTS);
            }
        }
    }
    if imported_any {
        let threads = state.threads;
        let run = with_store(&state.store, move |s| {
            s.optimize(false, "watch folder", threads)
        })
        .await;
        if let Ok(run) = run {
            println!("watch folder: {}", run.message);
        }
    }
    if let Ok(mut w) = state.watch.lock() {
        w.last_scan = Some(now());
        w.files_seen = handled.len();
    }
}

fn set_error(state: &AppState, error: Option<String>) {
    if let Ok(mut w) = state.watch.lock() {
        w.error = error;
        w.last_scan = Some(now());
    }
}
