//! The master database: opening it, importing files, running the optimiser and
//! reading back the current state of every pile and tracker.

use crate::dates::find_date;
use crate::report::{ImportReport, Level};
use crate::settings::{FloodMode, Settings};
use crate::table::{Col, Table, norm_header, parse_num, read_table};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use top_core::{Pile, Tracker, default_solver, normalize_block, optimize_all};

pub type Result<T> = std::result::Result<T, String>;

/// A tracker's latest run.
struct LatestRun {
    run_id: i64,
    run_at: String,
    status: String,
    sig: String,
    max_angle: f64,
    max_cut_fill: f64,
    max_shift: Option<f64>,
    span_error: Option<f64>,
    anchor_pile: Option<String>,
    anchor_motor: Option<bool>,
}

const SCHEMA: &str = include_str!("schema.sql");
const SCHEMA_VERSION: &str = "4";

fn db(e: rusqlite::Error) -> String {
    match e {
        rusqlite::Error::SqliteFailure(_, Some(msg)) => msg,
        other => format!("database error: {other}"),
    }
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Master,
    Survey,
    Lidar,
    Flood,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Master => "master",
            Kind::Survey => "survey",
            Kind::Lidar => "lidar",
            Kind::Flood => "flood",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "master" => Some(Kind::Master),
            "survey" => Some(Kind::Survey),
            "lidar" => Some(Kind::Lidar),
            "flood" => Some(Kind::Flood),
            _ => None,
        }
    }
}

/// One row of the import history.
#[derive(Clone, Debug, Serialize)]
pub struct ImportRow {
    pub import_id: i64,
    pub kind: String,
    pub file_name: String,
    pub imported_at: String,
    pub survey_date: Option<String>,
    pub origin: String,
    pub rows_total: i64,
    pub rows_used: i64,
    pub rows_rejected: i64,
    pub voided_at: Option<String>,
    pub void_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<ImportReport>,
}

/// Latest result of a pile.
#[derive(Clone, Debug, Serialize)]
pub struct PileResultRow {
    pub run_id: i64,
    /// Ground elevation the run used.
    pub z_ground: f64,
    pub new_ground: f64,
    pub cut_fill: f64,
    pub top: f64,
    pub reveal: f64,
    pub angle_diff_deg: f64,
    pub outlier: bool,
    /// Lowest allowed top the run used (flood level + clearance).
    pub min_top: Option<f64>,
    /// The top sits on that minimum: the pile was raised for flood.
    pub min_top_active: bool,
    /// Set-out position and move along the tracker from the design position.
    pub set_easting: Option<f64>,
    pub set_northing: Option<f64>,
    pub shift: Option<f64>,
}

/// Everything known about one pile right now.
#[derive(Clone, Debug, Serialize)]
pub struct PileState {
    pub pile_id: String,
    pub tracker: String,
    pub block: String,
    pub section: String,
    pub easting: f64,
    pub northing: f64,
    pub pile_type: String,
    pub color: String,
    pub tracker_type: String,
    /// Ground elevation in use: the latest field survey, else the latest LIDAR.
    pub z: Option<f64>,
    pub z_source: Option<Kind>,
    pub z_import_id: Option<i64>,
    pub z_date: Option<String>,
    /// Latest field-survey ground (and its date) and latest LIDAR / design
    /// ground, whichever is in use.
    pub z_survey: Option<f64>,
    pub z_survey_date: Option<String>,
    pub z_lidar: Option<f64>,
    /// Surveyed position minus design position, horizontal distance (m).
    pub offset_m: Option<f64>,
    /// How many field surveys measured this pile (voided ones excluded).
    pub n_surveys: u32,
    /// Latest flood value (see `Settings::flood_mode`), and its import.
    pub flood: Option<f64>,
    pub flood_import_id: Option<i64>,
    /// Flood water level and lowest allowed top, with the current ground and settings.
    pub flood_level: Option<f64>,
    pub min_top: Option<f64>,
    pub result: Option<PileResultRow>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TrackerState {
    pub tracker: String,
    pub block: String,
    pub section: String,
    pub tracker_type: String,
    pub n_piles: usize,
    pub n_survey: usize,
    pub n_lidar: usize,
    pub n_missing: usize,
    /// Compliant, Graded, NonCompliant, Skipped, InvalidInput (from the latest run),
    /// or AwaitingSurvey / NotRun when it has never been optimised.
    pub status: String,
    /// The latest run used other elevations or settings than the current ones.
    pub stale: bool,
    pub run_id: Option<i64>,
    pub run_at: Option<String>,
    pub max_angle_deg: Option<f64>,
    pub max_cut_fill: Option<f64>,
    /// Set-out of the latest result: largest move along the tracker (m),
    /// largest error of a span's length along the tube (m), the pile kept fixed.
    pub max_shift: Option<f64>,
    pub span_error: Option<f64>,
    pub anchor_pile: Option<String>,
    pub anchor_motor: Option<bool>,
    /// Piles with a flood value, and piles raised by it in the latest result.
    pub n_flood: usize,
    pub n_flood_raised: usize,
    /// Index of this tracker's first pile in `State::piles`.
    pub first_pile: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct State {
    /// Sorted by tracker, then northing.
    pub piles: Vec<PileState>,
    pub trackers: Vec<TrackerState>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RunSummary {
    pub run_id: Option<i64>,
    pub created_at: String,
    pub trigger: String,
    pub trackers_run: usize,
    pub up_to_date: usize,
    pub awaiting: usize,
    pub counts: BTreeMap<String, usize>,
    pub duration_ms: u64,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlockSummary {
    pub block: String,
    pub piles: usize,
    pub surveyed: usize,
    pub trackers: usize,
    pub counts: BTreeMap<String, usize>,
    /// Mean reveal of the block's piles that have a result (m).
    pub reveal_mean: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub db_path: String,
    pub piles: usize,
    pub trackers: usize,
    pub piles_surveyed: usize,
    pub piles_lidar_only: usize,
    pub piles_no_elevation: usize,
    pub piles_with_flood: usize,
    pub piles_flood_raised: usize,
    /// Reveal (top of pile above ground) of the piles with a result: mean,
    /// mean of those whose ground comes from a field survey, lowest, highest.
    pub piles_with_result: usize,
    pub reveal_mean: Option<f64>,
    pub reveal_mean_survey: Option<f64>,
    pub reveal_min: Option<f64>,
    pub reveal_max: Option<f64>,
    pub tracker_status: BTreeMap<String, usize>,
    pub stale_trackers: usize,
    pub imports: usize,
    pub last_survey: Option<ImportRow>,
    pub last_run: Option<RunSummary>,
    pub blocks: Vec<BlockSummary>,
}

/// Latest ground of a pile by source.
#[derive(Default)]
struct GroundKinds {
    survey: Option<(f64, String)>,
    lidar: Option<f64>,
}

/// Latest elevation of a pile.
#[derive(Clone, Debug)]
struct Elev {
    z: f64,
    import_id: i64,
    kind: Kind,
    date: String,
    offset_m: Option<f64>,
}

struct MasterPile {
    pile_id: String,
    tracker: String,
    block: String,
    section: String,
    easting: f64,
    northing: f64,
    pile_type: String,
    color: String,
    tracker_type: String,
}

/// Lookup of master piles by ID and by position.
struct PileIndex<'a> {
    piles: &'a [MasterPile],
    by_id: HashMap<&'a str, usize>,
    by_upper: HashMap<String, usize>,
    grid: HashMap<(i64, i64), Vec<usize>>,
    cell: f64,
}

impl<'a> PileIndex<'a> {
    fn new(piles: &'a [MasterPile], radius: f64) -> Self {
        let cell = radius.max(0.25);
        let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, p) in piles.iter().enumerate() {
            grid.entry(Self::key(cell, p.easting, p.northing))
                .or_default()
                .push(i);
        }
        Self {
            piles,
            by_id: piles
                .iter()
                .enumerate()
                .map(|(i, p)| (p.pile_id.as_str(), i))
                .collect(),
            by_upper: piles
                .iter()
                .enumerate()
                .map(|(i, p)| (p.pile_id.to_uppercase(), i))
                .collect(),
            grid,
            cell,
        }
    }

    fn key(cell: f64, e: f64, n: f64) -> (i64, i64) {
        ((e / cell).floor() as i64, (n / cell).floor() as i64)
    }

    fn by_id(&self, id: &str) -> Option<usize> {
        self.by_id
            .get(id)
            .copied()
            .or_else(|| self.by_upper.get(&id.to_uppercase()).copied())
    }

    /// A pile ID in a cell: the whole cell, or one of its words, so that
    /// `"P01.02.01.05_TN"` or `"TN P01.02.01.05"` are found too.
    fn find_in_cell(&self, cell: &str) -> Option<usize> {
        let cell = cell.trim();
        if cell.is_empty() {
            return None;
        }
        self.by_id(cell).or_else(|| {
            cell.split(|c: char| {
                c.is_whitespace()
                    || matches!(
                        c,
                        '_' | ';' | ',' | '|' | '/' | '\\' | '(' | ')' | '[' | ']'
                    )
            })
            .filter(|t| !t.is_empty() && *t != cell)
            .find_map(|t| self.by_id(t))
        })
    }

    fn nearest(&self, e: f64, n: f64, radius: f64) -> Option<(usize, f64)> {
        let (cx, cy) = Self::key(self.cell, e, n);
        let reach = (radius / self.cell).ceil() as i64;
        let mut best: Option<(usize, f64)> = None;
        for dx in -reach..=reach {
            for dy in -reach..=reach {
                for &i in self
                    .grid
                    .get(&(cx + dx, cy + dy))
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                {
                    let p = &self.piles[i];
                    let d = (p.easting - e).hypot(p.northing - n);
                    if d <= radius && best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((i, d));
                    }
                }
            }
        }
        best
    }
}

/// Tracker of a pile ID without a Tracker column: the ID minus its last part.
fn tracker_from_id(id: &str) -> Option<String> {
    let cut = id.rfind(['.', '-', '_', ' '])?;
    let t = id[..cut].trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn line_no(table: &Table, idx: usize) -> usize {
    if table.had_header { idx + 2 } else { idx + 1 }
}

pub struct Store {
    pub(crate) conn: Connection,
    path: PathBuf,
    /// Revision of the last copy handed out for the shared folder (see sync.rs).
    pub(crate) snapshot_revision: Option<String>,
}

impl Store {
    /// Open (or create) a database file.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(db)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(db)?;
        Self::init(conn, path.to_path_buf())
    }

    /// Open a database from a copy of its file (a backup, or the browser's
    /// stored copy). It lives in memory; `to_bytes` gives the file back.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut conn = Connection::open_in_memory().map_err(db)?;
        Self::deserialize(&mut conn, bytes)?;
        let is_ours: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')
                    AND EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'piles')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if !is_ours {
            return Err("this file is not a tracker-top database".into());
        }
        Self::init(conn, PathBuf::from("this browser"))
    }

    /// Load a database file's bytes into an in-memory connection.
    fn deserialize(conn: &mut Connection, bytes: &[u8]) -> Result<()> {
        if bytes.len() < 100 || &bytes[..16] != b"SQLite format 3\0" {
            return Err("this file is not a tracker-top database".into());
        }
        let mut data = bytes.to_vec();
        // A copy in memory uses a rollback journal, not WAL (header bytes 18-19).
        data[18] = 1;
        data[19] = 1;
        conn.deserialize_read_exact("main", &data[..], data.len(), false)
            .map_err(|e| format!("cannot read the database file: {e}"))
    }

    /// Replace this in-memory database with the database in `bytes`.
    pub(crate) fn load_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        Self::deserialize(&mut self.conn, bytes)?;
        self.conn
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(db)
    }

    /// The whole database as the bytes of a database file.
    pub fn to_bytes(&mut self) -> Result<Vec<u8>> {
        Ok(self.conn.serialize("main").map_err(db)?.to_vec())
    }

    /// Name shown for where the database lives.
    pub fn set_label(&mut self, label: &str) {
        self.path = PathBuf::from(label);
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(
            Connection::open_in_memory().map_err(db)?,
            PathBuf::from(":memory:"),
        )
    }

    fn init(conn: Connection, path: PathBuf) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON").map_err(db)?;
        // Check the version before touching anything: the meta table is missing in a new file.
        let existing: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .ok();
        match existing.as_deref() {
            None | Some(SCHEMA_VERSION) | Some("2") | Some("3") => {}
            Some("1") => {
                return Err("this database was made by a preview version of tracker-top (schema 1) \
                     and cannot be opened: start a new database and import the master pile list again"
                    .into());
            }
            Some(v) => {
                return Err(format!(
                    "database schema version {v} is not supported by this program (it expects {SCHEMA_VERSION}); \
                     use a newer version of tracker-top"
                ));
            }
        }
        conn.execute_batch(SCHEMA).map_err(db)?;
        if existing.as_deref() == Some("2") {
            // 2 -> 3: set-out results. Nothing is lost; trackers are re-optimised
            // on the next run because the settings signature changes.
            conn.execute_batch(
                "BEGIN;
                 ALTER TABLE run_piles ADD COLUMN set_e REAL;
                 ALTER TABLE run_piles ADD COLUMN set_n REAL;
                 ALTER TABLE run_piles ADD COLUMN shift REAL;
                 ALTER TABLE run_trackers ADD COLUMN max_shift REAL;
                 ALTER TABLE run_trackers ADD COLUMN span_error REAL;
                 ALTER TABLE run_trackers ADD COLUMN anchor_pile TEXT;
                 ALTER TABLE run_trackers ADD COLUMN anchor_motor INTEGER;
                 COMMIT;",
            )
            .map_err(db)?;
        }
        // 3 -> 4: the installer export log and the revision triggers, both
        // created by the schema above. Nothing else changes.
        let store = Self {
            conn,
            path,
            snapshot_revision: None,
        };
        if existing.as_deref() != Some(SCHEMA_VERSION) {
            store.set_meta("schema_version", SCHEMA_VERSION)?;
        }
        if store.meta("revision")?.is_none() {
            store.set_meta("revision", &crate::sync::new_revision())?;
        }
        if existing.as_deref() == Some("2") {
            // The site is in GDA2020 / MGA zone 56: fill the ArcGIS export's
            // coordinate system if it was never set.
            let mut s = store.settings()?;
            if s.epsg.is_none() {
                s.epsg = Settings::default().epsg;
                store.set_settings(&s)?;
            }
        }
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(db)
    }

    pub(crate) fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map(|_| ())
            .map_err(db)
    }

    pub fn settings(&self) -> Result<Settings> {
        match self.meta("settings")? {
            Some(json) => {
                let mut s: Settings = serde_json::from_str(&json)
                    .map_err(|e| format!("stored settings are invalid: {e}"))?;
                // Saved before the flood rule (pile height, module clearance,
                // freeboard): flood values are now water levels by default.
                if !json.contains("\"flood_freeboard_m\"") {
                    s.flood_mode = crate::FloodMode::Elevation;
                }
                Ok(s)
            }
            None => Ok(Settings::default()),
        }
    }

    pub fn set_settings(&self, s: &Settings) -> Result<()> {
        s.validate()?;
        if self.settings()? == *s && self.meta("settings")?.is_some() {
            return Ok(()); // unchanged: no new revision to share
        }
        self.set_meta(
            "settings",
            &serde_json::to_string(s).expect("settings serialise"),
        )?;
        // When, so that the newest settings win when two computers' changes meet.
        self.set_meta("settings_at", &crate::sync::utc_now())
    }

    /// Folder scanned for new survey files (e.g. a Trimble Connect Sync folder).
    pub fn watch_folder(&self) -> Result<Option<String>> {
        Ok(self.meta("watch_folder")?.filter(|s| !s.trim().is_empty()))
    }

    pub fn set_watch_folder(&self, folder: Option<&str>) -> Result<()> {
        self.set_meta("watch_folder", folder.map(str::trim).unwrap_or(""))
    }

    pub fn pile_count(&self) -> Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM piles", [], |r| r.get::<_, i64>(0))
            .map(|n| n as usize)
            .map_err(db)
    }

    /// True if a file with this content was ever imported as `kind`, voided or not.
    pub fn file_seen(&self, kind: Kind, sha: &str) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM imports WHERE kind = ?1 AND sha256 = ?2)",
                params![kind.as_str(), sha],
                |r| r.get(0),
            )
            .map_err(db)
    }

    fn master_piles(&self) -> Result<Vec<MasterPile>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT pile_id, tracker, block, section, easting, northing, pile_type, color, tracker_type
                 FROM piles ORDER BY tracker, northing, pile_id",
            )
            .map_err(db)?;
        st.query_map([], |r| {
            Ok(MasterPile {
                pile_id: r.get(0)?,
                tracker: r.get(1)?,
                block: r.get(2)?,
                section: r.get(3)?,
                easting: r.get(4)?,
                northing: r.get(5)?,
                pile_type: r.get(6)?,
                color: r.get(7)?,
                tracker_type: r.get(8)?,
            })
        })
        .map_err(db)?
        .collect::<rusqlite::Result<_>>()
        .map_err(db)
    }

    /// Latest non-voided elevation per pile: field survey first, then LIDAR;
    /// within a kind the most recent survey date, then the most recent import.
    fn latest_elevations(&self) -> Result<HashMap<String, Elev>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT e.pile_id, e.z, e.import_id, i.kind,
                        COALESCE(i.survey_date, substr(i.imported_at, 1, 10)), e.offset_m
                 FROM elevations e JOIN imports i ON i.import_id = e.import_id
                 WHERE i.voided_at IS NULL
                 ORDER BY e.pile_id,
                          CASE i.kind WHEN 'survey' THEN 0 ELSE 1 END,
                          COALESCE(i.survey_date, substr(i.imported_at, 1, 10)) DESC,
                          e.import_id DESC",
            )
            .map_err(db)?;
        let mut out = HashMap::new();
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    Elev {
                        z: r.get(1)?,
                        import_id: r.get(2)?,
                        kind: Kind::parse(&r.get::<_, String>(3)?).unwrap_or(Kind::Lidar),
                        date: r.get(4)?,
                        offset_m: r.get(5)?,
                    },
                ))
            })
            .map_err(db)?;
        for row in rows {
            let (id, e) = row.map_err(db)?;
            out.entry(id).or_insert(e);
        }
        Ok(out)
    }

    /// Latest survey ground (with its date) and latest LIDAR ground per pile.
    fn ground_by_kind(&self) -> Result<HashMap<String, GroundKinds>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT e.pile_id, e.z, i.kind, COALESCE(i.survey_date, substr(i.imported_at, 1, 10))
                 FROM elevations e JOIN imports i ON i.import_id = e.import_id
                 WHERE i.voided_at IS NULL
                 ORDER BY e.pile_id,
                          COALESCE(i.survey_date, substr(i.imported_at, 1, 10)) DESC,
                          e.import_id DESC",
            )
            .map_err(db)?;
        let mut out: HashMap<String, GroundKinds> = HashMap::new();
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(db)?;
        for row in rows {
            let (id, z, kind, date) = row.map_err(db)?;
            let g = out.entry(id).or_default();
            if kind == "survey" {
                g.survey.get_or_insert((z, date));
            } else {
                g.lidar.get_or_insert(z);
            }
        }
        Ok(out)
    }

    /// Latest non-voided flood value per pile: pile -> (value, import_id).
    fn latest_flood(&self) -> Result<HashMap<String, (f64, i64)>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT f.pile_id, f.value, f.import_id
                 FROM flood_values f JOIN imports i ON i.import_id = f.import_id
                 WHERE i.voided_at IS NULL
                 ORDER BY f.pile_id, f.import_id DESC",
            )
            .map_err(db)?;
        let rows = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))
            .map_err(db)?;
        let mut out = HashMap::new();
        for row in rows {
            let (id, v) = row.map_err(db)?;
            out.entry(id).or_insert(v);
        }
        Ok(out)
    }

    fn existing_import(&self, kind: Kind, sha: &str) -> Result<Option<(i64, String)>> {
        self.conn
            .query_row(
                "SELECT import_id, imported_at FROM imports WHERE kind = ?1 AND sha256 = ?2 AND voided_at IS NULL",
                params![kind.as_str(), sha],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db)
    }

    /// Import a file. User-level problems (bad columns, unknown piles, duplicate
    /// file...) come back in the report with `accepted == false`; `Err` means
    /// the database itself failed.
    pub fn import_file(
        &mut self,
        kind: Kind,
        file_name: &str,
        bytes: &[u8],
        survey_date: Option<&str>,
        origin: &str,
    ) -> Result<ImportReport> {
        let mut rep = ImportReport::new(kind.as_str(), file_name);
        let sha = sha256_hex(bytes);
        if let Some((id, at)) = self.existing_import(kind, &sha)? {
            rep.error(&format!(
                "This file was already imported on {at} (import #{id}). Void that import first to import it again."
            ));
            return Ok(rep);
        }
        let table = match read_table(file_name, bytes) {
            Ok(t) => t,
            Err(e) => {
                rep.error(&e);
                return Ok(rep);
            }
        };
        rep.rows_total = table.rows.len();
        if table.rows.is_empty() {
            rep.error("The file contains no data rows.");
            return Ok(rep);
        }
        if !table.had_header {
            rep.info("No header row: columns were recognised from their values.");
        }
        let mut rep = match kind {
            Kind::Master => self.import_master(rep, &table, &sha, origin)?,
            Kind::Survey | Kind::Lidar | Kind::Flood => {
                self.import_elevations(rep, kind, &table, &sha, survey_date, origin)?
            }
        };
        rep.sort();
        Ok(rep)
    }

    fn import_master(
        &mut self,
        mut rep: ImportReport,
        table: &Table,
        sha: &str,
        origin: &str,
    ) -> Result<ImportReport> {
        let c_id = table.find(Col::PileId);
        let c_e = table.find(Col::Easting);
        let c_n = table.find(Col::Northing);
        let c_tracker = table.find(Col::Tracker);
        let c_block = table.find(Col::Block);
        let c_section = table.find(Col::Section);
        let c_type = table.find(Col::PileType);
        let c_color = table.find(Col::Color);
        let c_ttype = table.find(Col::TrackerType);
        let c_z = table.find(Col::Elevation);
        let c_flood = table.find(Col::Flood);
        for (col, idx) in [
            (Col::PileId, c_id),
            (Col::Tracker, c_tracker),
            (Col::Block, c_block),
            (Col::Section, c_section),
            (Col::Easting, c_e),
            (Col::Northing, c_n),
            (Col::PileType, c_type),
            (Col::Color, c_color),
            (Col::TrackerType, c_ttype),
            (Col::Elevation, c_z),
            (Col::Flood, c_flood),
        ] {
            if let Some(i) = idx {
                rep.columns
                    .insert(col.label().into(), table.headers[i].clone());
            }
        }
        let missing: Vec<&str> = [
            (Col::PileId, c_id),
            (Col::Easting, c_e),
            (Col::Northing, c_n),
        ]
        .iter()
        .filter(|(_, c)| c.is_none())
        .map(|(col, _)| col.label())
        .collect();
        if !missing.is_empty() {
            rep.error(&format!(
                "Required column(s) not found: {}. Headers in the file: {}.",
                missing.join(", "),
                table.headers.join(", ")
            ));
            return Ok(rep);
        }
        if c_tracker.is_none() {
            rep.info("No Tracker column: each pile's tracker is its ID without the last part (P01.02.01.05 -> P01.02.01).");
        }
        if c_color.is_none() {
            rep.info("No Color column: pile colours are left empty.");
        }
        if let Some(z) = c_z {
            let others: Vec<&str> = table
                .find_all(Col::Elevation)
                .into_iter()
                .filter(|&i| i != z)
                .map(|i| table.headers[i].as_str())
                .collect();
            if !others.is_empty() {
                rep.info(&format!(
                    "Ground elevation read from \"{}\"; also in the file but not used: {}.",
                    table.headers[z],
                    others.join(", ")
                ));
            }
        }

        let mut rows: Vec<(MasterPile, Option<f64>, Option<f64>)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (i, row) in table.rows.iter().enumerate() {
            let ln = line_no(table, i);
            let id = table.cell(row, c_id).to_string();
            if id.is_empty() {
                rep.add(Level::Error, "Pile ID is empty", format!("line {ln}"));
                continue;
            }
            if !seen.insert(id.clone()) {
                rep.add(
                    Level::Error,
                    "Pile ID appears more than once",
                    format!("line {ln}: {id}"),
                );
                continue;
            }
            let (Some(e), Some(n)) = (
                parse_num(table.cell(row, c_e)),
                parse_num(table.cell(row, c_n)),
            ) else {
                rep.add(
                    Level::Error,
                    "Easting/Northing missing or not a number",
                    format!("line {ln}: {id}"),
                );
                continue;
            };
            let tracker = match c_tracker
                .map(|c| table.cell(row, Some(c)))
                .filter(|t| !t.is_empty())
            {
                Some(t) => t.to_string(),
                None => match tracker_from_id(&id) {
                    Some(t) => t,
                    None => {
                        rep.add(
                            Level::Error,
                            "Tracker unknown (no Tracker value and the ID has no separator)",
                            format!("line {ln}: {id}"),
                        );
                        continue;
                    }
                },
            };
            let z = parse_num(table.cell(row, c_z));
            let flood = parse_num(table.cell(row, c_flood));
            rows.push((
                MasterPile {
                    pile_id: id,
                    tracker,
                    block: normalize_block(table.cell(row, c_block)),
                    section: table.cell(row, c_section).to_string(),
                    easting: e,
                    northing: n,
                    pile_type: table.cell(row, c_type).to_string(),
                    color: table.cell(row, c_color).to_string(),
                    tracker_type: table.cell(row, c_ttype).to_string(),
                },
                z,
                flood,
            ));
        }
        if rep.has_errors() {
            rep.rows_rejected = rep.rows_total - rows.len();
            rep.add(Level::Error, "The master list must be clean: nothing was imported. Fix the rows above and import again.", "");
            return Ok(rep);
        }

        // The core is immutable: existing piles must be identical, new ones are added.
        let existing: HashMap<String, MasterPile> = self
            .master_piles()?
            .into_iter()
            .map(|p| (p.pile_id.clone(), p))
            .collect();
        let mut new_rows = Vec::new();
        let mut same = 0usize;
        for (p, z, flood) in rows {
            match existing.get(&p.pile_id) {
                None => new_rows.push((p, z, flood)),
                Some(old) => {
                    let identical = old.tracker == p.tracker
                        && old.block == p.block
                        && old.section == p.section
                        && old.pile_type == p.pile_type
                        && old.color == p.color
                        && old.tracker_type == p.tracker_type
                        && (old.easting - p.easting).abs() < 0.001
                        && (old.northing - p.northing).abs() < 0.001;
                    if identical {
                        same += 1;
                    } else {
                        rep.add(
                            Level::Error,
                            "Pile already in the master list with different data (the master list cannot be changed)",
                            p.pile_id.clone(),
                        );
                    }
                }
            }
        }
        if rep.has_errors() {
            rep.add(Level::Error, "Nothing was imported.", "");
            return Ok(rep);
        }
        if new_rows.is_empty() {
            rep.error("Every pile in this file is already in the master list: nothing to add. To load ground elevations, import the file as LIDAR / design ground instead.");
            return Ok(rep);
        }
        if same > 0 {
            rep.add(Level::Info, &format!("{same} piles were already in the master list (identical) and were left as they are."), "");
        }
        if !existing.is_empty() {
            rep.add(
                Level::Warning,
                &format!(
                    "{} new piles were added to an existing master list.",
                    new_rows.len()
                ),
                "",
            );
        }

        let with_z = new_rows.iter().filter(|(_, z, _)| z.is_some()).count();
        let z_col = rep.columns.get("Elevation").cloned().unwrap_or_default();
        if with_z > 0 {
            rep.add(
                Level::Info,
                &format!("The file has a ground elevation column (\"{z_col}\"): {with_z} elevations were also recorded as LIDAR / design ground."),
                "",
            );
        }
        let with_flood = new_rows.iter().filter(|(_, _, f)| f.is_some()).count();
        let flood_col = rep.columns.get("Flood").cloned().unwrap_or_default();
        match (c_flood, with_flood) {
            (None, _) => rep.info("No flood column: flood values can be imported later (Import > Flood values)."),
            (Some(_), 0) => rep.info(&format!("The flood column (\"{flood_col}\") is empty: flood values can be imported later.")),
            (Some(_), n) => rep.add(
                Level::Info,
                &format!("The file has a flood column (\"{flood_col}\"): {n} flood values were also recorded."),
                "",
            ),
        }
        rep.sort();
        let at = now();
        rep.rows_used = new_rows.len();
        rep.rows_rejected = 0;
        rep.accepted = true;
        let tx = self.conn.transaction().map_err(db)?;
        let report_json = serde_json::to_string(&rep).expect("report serialises");
        tx.execute(
            "INSERT INTO imports (kind, file_name, sha256, imported_at, survey_date, origin, rows_total, rows_used, rows_rejected, report)
             VALUES ('master', ?1, ?2, ?3, NULL, ?4, ?5, ?6, 0, ?7)",
            params![rep.file_name, sha, at, origin, rep.rows_total as i64, rep.rows_used as i64, report_json],
        )
        .map_err(db)?;
        let master_id = tx.last_insert_rowid();
        {
            let mut ins = tx
                .prepare(
                    "INSERT INTO piles (pile_id, tracker, block, section, easting, northing, pile_type, color, tracker_type, import_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )
                .map_err(db)?;
            for (p, _, _) in &new_rows {
                ins.execute(params![
                    p.pile_id,
                    p.tracker,
                    p.block,
                    p.section,
                    p.easting,
                    p.northing,
                    p.pile_type,
                    p.color,
                    p.tracker_type,
                    master_id
                ])
                .map_err(db)?;
            }
        }
        if with_z > 0 {
            let mut lidar_rep = ImportReport::new("lidar", &rep.file_name);
            lidar_rep.accepted = true;
            lidar_rep.rows_total = rep.rows_total;
            lidar_rep.rows_used = with_z;
            lidar_rep.rows_rejected = new_rows.len() - with_z;
            lidar_rep.columns.insert("Elevation".into(), z_col.clone());
            lidar_rep.info(&format!("Ground elevations read from column \"{z_col}\" of the master pile list (import #{master_id})."));
            let lidar_report = serde_json::to_string(&lidar_rep).expect("report serialises");
            tx.execute(
                "INSERT INTO imports (kind, file_name, sha256, imported_at, survey_date, origin, rows_total, rows_used, rows_rejected, report)
                 VALUES ('lidar', ?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8)",
                params![rep.file_name, sha, at, origin, rep.rows_total as i64, with_z as i64, (new_rows.len() - with_z) as i64, lidar_report],
            )
            .map_err(db)?;
            let lidar_id = tx.last_insert_rowid();
            let mut ins = tx
                .prepare("INSERT INTO elevations (import_id, pile_id, z, matched_by) VALUES (?1, ?2, ?3, 'id')")
                .map_err(db)?;
            for (p, z, _) in &new_rows {
                if let Some(z) = z {
                    ins.execute(params![lidar_id, p.pile_id, z]).map_err(db)?;
                }
            }
        }
        if with_flood > 0 {
            let mut flood_rep = ImportReport::new("flood", &rep.file_name);
            flood_rep.accepted = true;
            flood_rep.rows_total = rep.rows_total;
            flood_rep.rows_used = with_flood;
            flood_rep.rows_rejected = new_rows.len() - with_flood;
            flood_rep.columns.insert("Flood".into(), flood_col.clone());
            flood_rep.info(&format!("Flood values read from column \"{flood_col}\" of the master pile list (import #{master_id})."));
            tx.execute(
                "INSERT INTO imports (kind, file_name, sha256, imported_at, survey_date, origin, rows_total, rows_used, rows_rejected, report)
                 VALUES ('flood', ?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8)",
                params![
                    rep.file_name,
                    sha,
                    at,
                    origin,
                    rep.rows_total as i64,
                    with_flood as i64,
                    (new_rows.len() - with_flood) as i64,
                    serde_json::to_string(&flood_rep).expect("report serialises")
                ],
            )
            .map_err(db)?;
            let flood_id = tx.last_insert_rowid();
            let mut ins = tx
                .prepare("INSERT INTO flood_values (import_id, pile_id, value, matched_by) VALUES (?1, ?2, ?3, 'id')")
                .map_err(db)?;
            for (p, _, f) in &new_rows {
                if let Some(f) = f {
                    ins.execute(params![flood_id, p.pile_id, f]).map_err(db)?;
                }
            }
        }
        tx.commit().map_err(db)?;
        rep.import_id = Some(master_id);
        Ok(rep)
    }

    fn import_elevations(
        &mut self,
        mut rep: ImportReport,
        kind: Kind,
        table: &Table,
        sha: &str,
        survey_date: Option<&str>,
        origin: &str,
    ) -> Result<ImportReport> {
        let masters = self.master_piles()?;
        if masters.is_empty() {
            rep.error("Import the master pile list first: survey points are matched against it.");
            return Ok(rep);
        }
        let s = self.settings()?;
        let flood = kind == Kind::Flood;
        // The value read per pile: ground elevation, or flood value.
        let value_col = if flood { Col::Flood } else { Col::Elevation };
        let what = if flood { "Flood value" } else { "Elevation" };
        let index = PileIndex::new(&masters, s.match_radius_m);
        let current = if flood {
            HashMap::new()
        } else {
            self.latest_elevations()?
        };

        // Columns are found by header name first; their values then confirm or
        // correct them, so neither the column order nor the names matter.
        let mut c_id = table.find(Col::PileId);
        let mut c_e = table.find(Col::Easting);
        let mut c_n = table.find(Col::Northing);
        let mut c_z = table.find(value_col);
        if flood {
            // A flood file may give both depths and levels: read the one the
            // settings ask for, and never take one for the other.
            let kind_of = |c: usize| {
                let h = norm_header(&table.headers[c]);
                if ["depth", "hauteur", "profondeur", "height"]
                    .iter()
                    .any(|k| h.contains(k))
                {
                    Some(FloodMode::Depth)
                } else if ["level", "elevation", "niveau", "cote", "phe", "wse", "z"]
                    .iter()
                    .any(|k| h.contains(k))
                {
                    Some(FloodMode::Elevation)
                } else {
                    None
                }
            };
            let cols = table.find_all(Col::Flood);
            let want = s.flood_mode;
            let name = |m: FloodMode| match m {
                FloodMode::Depth => "depth",
                FloodMode::Elevation => "level",
            };
            let pick = cols
                .iter()
                .copied()
                .find(|&c| kind_of(c) == Some(want))
                .or_else(|| cols.iter().copied().find(|&c| kind_of(c).is_none()));
            let other = cols
                .iter()
                .copied()
                .find(|&c| kind_of(c).is_some_and(|k| k != want));
            match (pick, other) {
                (Some(c), o) => {
                    c_z = Some(c);
                    if let Some(o) = o {
                        rep.info(&format!(
                            "Flood {} read from column \"{}\" (Settings > Flood); the flood {} column \"{}\" is not used.",
                            name(want),
                            table.headers[c],
                            name(if want == FloodMode::Depth { FloodMode::Elevation } else { FloodMode::Depth }),
                            table.headers[o]
                        ));
                    }
                }
                (None, Some(o)) => {
                    rep.error(&format!(
                        "This file gives flood {}s (column \"{}\") but Settings > Flood says flood values are {}s. \
                         Change the setting, or import a file with flood {}s.",
                        name(if want == FloodMode::Depth { FloodMode::Elevation } else { FloodMode::Depth }),
                        table.headers[o],
                        name(want),
                        name(want)
                    ));
                    return Ok(rep);
                }
                (None, None) => {}
            }
        }
        let c_code = table.find(Col::Code);
        let c_date = table.find(Col::Date);
        let width = table.headers.len();
        let header = |c: usize| table.headers[c].as_str();

        // Pile IDs: the column whose values match the most piles of the master list.
        let id_hits = |c: usize| {
            table
                .rows
                .iter()
                .filter(|r| index.find_in_cell(table.cell(r, Some(c))).is_some())
                .count()
        };
        let header_hits = c_id.map(id_hits).unwrap_or(0);
        if let Some((best, hits)) = (0..width)
            .map(|c| (c, id_hits(c)))
            .max_by_key(|&(c, h)| (h, std::cmp::Reverse(c)))
            && hits > header_hits
        {
            rep.info(&format!(
                "Pile IDs recognised in column \"{}\" from their values ({hits} of {} rows match the master list).",
                header(best),
                table.rows.len()
            ));
            c_id = Some(best);
        }

        // Numeric columns and their medians (the ID column excluded).
        let numeric: Vec<(usize, f64)> = (0..width)
            .filter(|&c| Some(c) != c_id)
            .filter_map(|c| {
                let cells: Vec<&str> = table
                    .rows
                    .iter()
                    .map(|r| table.cell(r, Some(c)))
                    .filter(|v| !v.is_empty())
                    .collect();
                let values: Vec<f64> = cells.iter().filter_map(|v| parse_num(v)).collect();
                (!cells.is_empty() && values.len() * 10 >= cells.len() * 8)
                    .then(|| median(values))
                    .flatten()
                    .map(|m| (c, m))
            })
            .collect();
        let med_of = |c: Option<usize>| {
            c.and_then(|c| numeric.iter().find(|(k, _)| *k == c).map(|(_, m)| *m))
        };

        // Easting / Northing. With pile IDs: the columns whose values equal the
        // matched piles' design coordinates (within the reject distance).
        // Without: the columns whose typical value is nearest the site's.
        let matched_rows: Vec<(&Vec<String>, usize)> = c_id
            .map(|c| {
                table
                    .rows
                    .iter()
                    .filter_map(|r| index.find_in_cell(table.cell(r, Some(c))).map(|p| (r, p)))
                    .collect()
            })
            .unwrap_or_default();
        let tol = s.offset_reject_m.max(0.5);
        let axis_hits = |c: usize, east: bool| {
            matched_rows
                .iter()
                .filter(|(r, p)| {
                    let target = if east {
                        masters[*p].easting
                    } else {
                        masters[*p].northing
                    };
                    parse_num(table.cell(r, Some(c))).is_some_and(|v| (v - target).abs() <= tol)
                })
                .count()
        };
        let detected: Option<(usize, usize)> = if !matched_rows.is_empty() {
            let best = |east: bool| {
                numeric
                    .iter()
                    .map(|(c, _)| (*c, axis_hits(*c, east)))
                    .filter(|&(_, h)| h > 0)
                    .max_by_key(|&(c, h)| (h, std::cmp::Reverse(c)))
            };
            let (he, hn) = (
                c_e.map(|c| axis_hits(c, true)).unwrap_or(0),
                c_n.map(|c| axis_hits(c, false)).unwrap_or(0),
            );
            match (best(true), best(false)) {
                (Some((de, be)), Some((dn, bn))) if de != dn && (be > he || bn > hn) => {
                    Some((de, dn))
                }
                _ => None,
            }
        } else if let (Some(me), Some(mn)) = (
            median(masters.iter().map(|p| p.easting).collect()),
            median(masters.iter().map(|p| p.northing).collect()),
        ) {
            let fits = |m: f64, target: f64, other: f64| {
                (m - target).abs() < 20_000.0 && (m - target).abs() < (m - other).abs()
            };
            let best_for = |target: f64, other: f64| {
                numeric
                    .iter()
                    .filter(|(_, m)| fits(*m, target, other))
                    .min_by(|a, b| (a.1 - target).abs().total_cmp(&(b.1 - target).abs()))
                    .map(|(c, _)| *c)
            };
            let header_ok = med_of(c_e).is_some_and(|m| fits(m, me, mn))
                && med_of(c_n).is_some_and(|m| fits(m, mn, me));
            match (best_for(me, mn), best_for(mn, me)) {
                (Some(de), Some(dn)) if !header_ok && de != dn => Some((de, dn)),
                _ => None,
            }
        } else {
            None
        };
        if let Some((de, dn)) = detected {
            let swapped = Some(de) == c_n && Some(dn) == c_e;
            rep.info(&format!(
                "{}Easting read from \"{}\" and Northing from \"{}\", recognised from their values.",
                if swapped {
                    "Easting and Northing columns were swapped (X/Y convention): "
                } else {
                    ""
                },
                header(de),
                header(dn)
            ));
            c_e = Some(de);
            c_n = Some(dn);
        }

        // The value column when no header names it: by its values.
        if c_z.is_none() {
            let candidates: Vec<(usize, f64)> = numeric
                .iter()
                .filter(|(c, _)| Some(*c) != c_e && Some(*c) != c_n)
                .copied()
                .collect();
            let z_ref = median(current.values().map(|e| e.z).collect());
            let after = c_e.max(c_n);
            let pick = z_ref
                .and_then(|zr| {
                    candidates
                        .iter()
                        .filter(|(_, m)| (m - zr).abs() < 300.0)
                        .min_by(|a, b| (a.1 - zr).abs().total_cmp(&(b.1 - zr).abs()))
                        .map(|(c, _)| *c)
                })
                .or_else(|| {
                    candidates
                        .iter()
                        .find(|(c, _)| after.is_some_and(|a| *c > a))
                        .map(|(c, _)| *c)
                })
                .or_else(|| candidates.first().map(|(c, _)| *c));
            if let Some(c) = pick {
                rep.info(&format!(
                    "{what} read from column \"{}\", recognised from its values.",
                    header(c)
                ));
                c_z = Some(c);
            }
        }

        for (col, idx) in [
            (Col::PileId, c_id),
            (Col::Easting, c_e),
            (Col::Northing, c_n),
            (value_col, c_z),
            (Col::Code, c_code),
            (Col::Date, c_date),
        ] {
            if let Some(i) = idx {
                rep.columns
                    .insert(col.label().into(), table.headers[i].clone());
            }
        }
        if c_z.is_none() {
            rep.error(&format!(
                "No {} column found (expected a header such as {}). Headers in the file: {}.",
                what.to_lowercase(),
                if flood {
                    "Flood, Flood depth, Flood level or Inondation"
                } else {
                    "Elevation, Z, Height, Altitude or Z LIDAR"
                },
                table.headers.join(", ")
            ));
            return Ok(rep);
        }
        if let (Kind::Survey, Some(z)) = (kind, c_z)
            && crate::table::norm_header(&table.headers[z]).contains("lidar")
        {
            rep.error(&format!(
                    "The elevation column is \"{}\": this looks like LIDAR data, not a field survey. Import it as LIDAR / design ground instead.",
                    table.headers[z]
                ));
            return Ok(rep);
        }
        let has_xy = c_e.is_some() && c_n.is_some();
        if c_id.is_none() && !has_xy {
            rep.error("Points cannot be matched to piles: no column holds pile IDs of the master list, and no Easting and Northing columns were found.");
            return Ok(rep);
        }
        if !has_xy {
            // Normal for LIDAR and flood tables; worth a warning for a field survey.
            let level = if kind == Kind::Survey {
                Level::Warning
            } else {
                Level::Info
            };
            rep.add(level, "No Easting/Northing columns: points are matched by ID only and positions cannot be checked.", "");
        }

        let previous = if kind == Kind::Survey {
            current
        } else {
            HashMap::new()
        };

        struct Row {
            pile: usize,
            z: f64,
            e: Option<f64>,
            n: Option<f64>,
            offset: Option<f64>,
            by_position: bool,
            code: String,
        }
        let mut used: HashMap<usize, Row> = HashMap::new();
        let mut rejected = 0usize;
        let mut first_date: Option<String> = None;

        for (i, row) in table.rows.iter().enumerate() {
            let ln = line_no(table, i);
            let id = table.cell(row, c_id);
            let label = if id.is_empty() {
                format!("line {ln}")
            } else {
                format!("line {ln}: {id}")
            };
            if first_date.is_none() {
                first_date = find_date(table.cell(row, c_date));
            }
            let Some(z) = parse_num(table.cell(row, c_z)) else {
                rejected += 1;
                rep.add(
                    Level::Warning,
                    &format!("{what} missing or not a number: row skipped"),
                    format!("{label} ({:?})", table.cell(row, c_z)),
                );
                continue;
            };
            let e = parse_num(table.cell(row, c_e));
            let n = parse_num(table.cell(row, c_n));

            let mut matched = (!id.is_empty())
                .then(|| index.find_in_cell(id))
                .flatten()
                .map(|p| (p, false));
            if matched.is_none()
                && let (Some(e), Some(n)) = (e, n)
                && let Some((p, d)) = index.nearest(e, n, s.match_radius_m)
            {
                rep.add(
                    Level::Info,
                    &format!(
                        "ID not in the master list: matched to the nearest pile (within {} m)",
                        s.match_radius_m
                    ),
                    format!("{label} -> {} ({d:.2} m)", masters[p].pile_id),
                );
                matched = Some((p, true));
            }
            let Some((p, by_position)) = matched else {
                rejected += 1;
                let why = if has_xy {
                    format!(
                        "Not a pile: unknown ID and no pile within {} m (row skipped)",
                        s.match_radius_m
                    )
                } else {
                    "Not a pile: unknown ID (row skipped)".to_string()
                };
                rep.add(Level::Warning, &why, label);
                continue;
            };
            let pile = &masters[p];
            let offset = match (e, n) {
                (Some(e), Some(n)) => Some((pile.easting - e).hypot(pile.northing - n)),
                _ => None,
            };
            if let Some(d) = offset {
                if d > s.offset_reject_m {
                    rejected += 1;
                    rep.add(
                        Level::Warning,
                        &format!("Rejected: more than {} m from the pile's design position (wrong point ID?)", s.offset_reject_m),
                        format!("{label} is {d:.2} m away"),
                    );
                    continue;
                }
                if d > s.offset_warn_m {
                    rep.add(
                        Level::Warning,
                        &format!(
                            "Kept, but more than {} m from the pile's design position",
                            s.offset_warn_m
                        ),
                        format!("{} is {d:.2} m away", pile.pile_id),
                    );
                }
            }
            if let Some(prev) = previous.get(&pile.pile_id)
                && prev.kind == Kind::Survey
                && (z - prev.z).abs() > s.jump_warn_m
            {
                rep.add(
                        Level::Warning,
                        &format!("Kept, but the elevation changed by more than {} m since the previous survey", s.jump_warn_m),
                        format!("{}: {:.3} -> {:.3} ({:+.3} m)", pile.pile_id, prev.z, z, z - prev.z),
                    );
            }
            let code = table.cell(row, c_code).to_string();
            if used
                .insert(
                    p,
                    Row {
                        pile: p,
                        z,
                        e,
                        n,
                        offset,
                        by_position,
                        code,
                    },
                )
                .is_some()
            {
                rejected += 1;
                rep.add(
                    Level::Warning,
                    "Pile measured more than once in this file: the last row is used",
                    pile.pile_id.clone(),
                );
            }
        }

        rep.rows_used = used.len();
        rep.rows_rejected = rejected;
        if used.is_empty() {
            rep.error("No row could be matched to a pile: nothing was recorded.");
            return Ok(rep);
        }

        let (date, source) = if let Some(d) = survey_date.and_then(find_date) {
            (d, "entered")
        } else if let Some(d) = first_date {
            (d, "date column")
        } else if let Some(d) = find_date(&rep.file_name) {
            (d, "file name")
        } else {
            (
                chrono::Local::now().format("%Y-%m-%d").to_string(),
                "import day",
            )
        };
        rep.survey_date = Some(date.clone());
        rep.survey_date_source = Some(source.into());

        let trackers: HashSet<&str> = used
            .values()
            .map(|r| masters[r.pile].tracker.as_str())
            .collect();
        rep.add(
            Level::Info,
            &format!(
                "{} piles on {} trackers received a {} dated {date}.",
                used.len(),
                trackers.len(),
                match kind {
                    Kind::Survey => "surveyed elevation",
                    Kind::Flood => "flood value",
                    _ => "LIDAR elevation",
                }
            ),
            "",
        );

        rep.accepted = true;
        rep.sort();
        let tx = self.conn.transaction().map_err(db)?;
        tx.execute(
            "INSERT INTO imports (kind, file_name, sha256, imported_at, survey_date, origin, rows_total, rows_used, rows_rejected, report)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                kind.as_str(),
                rep.file_name,
                sha,
                now(),
                date,
                origin,
                rep.rows_total as i64,
                rep.rows_used as i64,
                rep.rows_rejected as i64,
                serde_json::to_string(&rep).expect("report serialises")
            ],
        )
        .map_err(db)?;
        let import_id = tx.last_insert_rowid();
        if flood {
            let mut ins = tx
                .prepare(
                    "INSERT INTO flood_values (import_id, pile_id, value, matched_by) VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(db)?;
            let mut rows: Vec<&Row> = used.values().collect();
            rows.sort_by_key(|r| r.pile);
            for r in rows {
                ins.execute(params![
                    import_id,
                    masters[r.pile].pile_id,
                    r.z,
                    if r.by_position { "position" } else { "id" }
                ])
                .map_err(db)?;
            }
        } else {
            let mut ins = tx
                .prepare(
                    "INSERT INTO elevations (import_id, pile_id, z, easting, northing, offset_m, matched_by, code)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .map_err(db)?;
            let mut rows: Vec<&Row> = used.values().collect();
            rows.sort_by_key(|r| r.pile);
            for r in rows {
                ins.execute(params![
                    import_id,
                    masters[r.pile].pile_id,
                    r.z,
                    r.e,
                    r.n,
                    r.offset,
                    if r.by_position { "position" } else { "id" },
                    r.code
                ])
                .map_err(db)?;
            }
        }
        tx.commit().map_err(db)?;
        rep.import_id = Some(import_id);
        Ok(rep)
    }

    fn import_row(r: &rusqlite::Row<'_>, with_report: bool) -> rusqlite::Result<ImportRow> {
        let report: String = r.get(11)?;
        Ok(ImportRow {
            import_id: r.get(0)?,
            kind: r.get(1)?,
            file_name: r.get(2)?,
            imported_at: r.get(3)?,
            survey_date: r.get(4)?,
            origin: r.get(5)?,
            rows_total: r.get(6)?,
            rows_used: r.get(7)?,
            rows_rejected: r.get(8)?,
            voided_at: r.get(9)?,
            void_reason: r.get(10)?,
            report: if with_report {
                serde_json::from_str(&report).ok()
            } else {
                None
            },
        })
    }

    const IMPORT_COLS: &'static str = "import_id, kind, file_name, imported_at, survey_date, origin, rows_total, rows_used, rows_rejected, voided_at, void_reason, report";

    /// Import history, newest first.
    pub fn imports(&self) -> Result<Vec<ImportRow>> {
        let mut st = self
            .conn
            .prepare(&format!(
                "SELECT {} FROM imports ORDER BY import_id DESC",
                Self::IMPORT_COLS
            ))
            .map_err(db)?;
        st.query_map([], |r| Self::import_row(r, false))
            .map_err(db)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db)
    }

    pub fn import_detail(&self, id: i64) -> Result<Option<ImportRow>> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {} FROM imports WHERE import_id = ?1",
                    Self::IMPORT_COLS
                ),
                [id],
                |r| Self::import_row(r, true),
            )
            .optional()
            .map_err(db)
    }

    /// Mark a survey or LIDAR import as void: its elevations stop being used,
    /// but stay in the database for the record.
    pub fn void_import(&mut self, id: i64, reason: &str) -> Result<()> {
        let Some(row) = self.import_detail(id)? else {
            return Err(format!("import #{id} does not exist"));
        };
        if row.kind == "master" {
            return Err(
                "the master pile list cannot be voided: it is the immutable core of the database"
                    .into(),
            );
        }
        if row.voided_at.is_some() {
            return Err(format!("import #{id} is already void"));
        }
        let reason = reason.trim();
        if reason.is_empty() {
            return Err("give a reason for voiding the import".into());
        }
        self.conn
            .execute(
                "UPDATE imports SET voided_at = ?1, void_reason = ?2 WHERE import_id = ?3",
                params![now(), reason, id],
            )
            .map(|_| ())
            .map_err(db)
    }

    /// Latest stored run of every tracker: tracker -> (run_id, created_at, status, sig, max angle, max cut/fill).
    ///
    /// A plain scan in run order, keeping the last row per tracker: each run only
    /// stores the trackers it re-optimised, so this stays small, and it avoids a
    /// grouped self-join that SQLite plans badly on large sites.
    fn latest_runs(&self) -> Result<HashMap<String, LatestRun>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT rt.tracker, rt.run_id, r.created_at, rt.status, rt.input_sig, rt.max_angle_deg, rt.max_cut_fill,
                        rt.max_shift, rt.span_error, rt.anchor_pile, rt.anchor_motor
                 FROM run_trackers rt JOIN runs r ON r.run_id = rt.run_id
                 ORDER BY rt.run_id",
            )
            .map_err(db)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    LatestRun {
                        run_id: r.get(1)?,
                        run_at: r.get(2)?,
                        status: r.get(3)?,
                        sig: r.get(4)?,
                        max_angle: r.get(5)?,
                        max_cut_fill: r.get(6)?,
                        max_shift: r.get(7)?,
                        span_error: r.get(8)?,
                        anchor_pile: r.get(9)?,
                        anchor_motor: r.get::<_, Option<i64>>(10)?.map(|v| v != 0),
                    },
                ))
            })
            .map_err(db)?;
        let mut out = HashMap::new();
        for row in rows {
            let (tracker, v) = row.map_err(db)?;
            out.insert(tracker, v);
        }
        Ok(out)
    }

    /// Result of every pile in its tracker's latest run. Reads each needed run
    /// by its primary key, keeping only the piles whose tracker's latest run it is.
    fn latest_pile_results(
        &self,
        masters: &[MasterPile],
        runs: &HashMap<String, LatestRun>,
    ) -> Result<HashMap<String, PileResultRow>> {
        let latest_of_pile: HashMap<&str, i64> = masters
            .iter()
            .filter_map(|p| runs.get(&p.tracker).map(|r| (p.pile_id.as_str(), r.run_id)))
            .collect();
        let mut run_ids: Vec<i64> = runs.values().map(|r| r.run_id).collect();
        run_ids.sort_unstable();
        run_ids.dedup();
        let mut st = self
            .conn
            .prepare(
                "SELECT pile_id, run_id, z_ground, new_ground, cut_fill, top, reveal, angle_diff_deg, outlier,
                        min_top, min_top_active, set_e, set_n, shift
                 FROM run_piles WHERE run_id = ?1",
            )
            .map_err(db)?;
        let mut out = HashMap::with_capacity(latest_of_pile.len());
        for run_id in run_ids {
            let rows = st
                .query_map([run_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        PileResultRow {
                            run_id: r.get(1)?,
                            z_ground: r.get(2)?,
                            new_ground: r.get(3)?,
                            cut_fill: r.get(4)?,
                            top: r.get(5)?,
                            reveal: r.get(6)?,
                            angle_diff_deg: r.get(7)?,
                            outlier: r.get::<_, i64>(8)? != 0,
                            min_top: r.get(9)?,
                            min_top_active: r.get::<_, i64>(10)? != 0,
                            set_easting: r.get(11)?,
                            set_northing: r.get(12)?,
                            shift: r.get(13)?,
                        },
                    ))
                })
                .map_err(db)?;
            for row in rows {
                let (pile, res) = row.map_err(db)?;
                if latest_of_pile.get(pile.as_str()) == Some(&run_id) {
                    out.insert(pile, res);
                }
            }
        }
        Ok(out)
    }

    fn survey_counts(&self) -> Result<HashMap<String, u32>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT e.pile_id, COUNT(*) FROM elevations e JOIN imports i ON i.import_id = e.import_id
                 WHERE i.kind = 'survey' AND i.voided_at IS NULL GROUP BY e.pile_id",
            )
            .map_err(db)?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(db)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db)
    }

    /// Signature of a tracker's optimisation inputs. Equal signatures give equal results.
    fn input_sig(
        params_json: &str,
        piles: &[MasterPile],
        elevs: &[&Elev],
        floods: &HashMap<String, (f64, i64)>,
    ) -> String {
        let mut h = Sha256::new();
        h.update(params_json.as_bytes());
        for (p, e) in piles.iter().zip(elevs) {
            h.update(p.pile_id.as_bytes());
            h.update([0]);
            h.update(p.easting.to_bits().to_le_bytes());
            h.update(p.northing.to_bits().to_le_bytes());
            h.update(e.z.to_bits().to_le_bytes());
            h.update(e.import_id.to_le_bytes());
            match floods.get(&p.pile_id) {
                Some((v, id)) => {
                    h.update([1]);
                    h.update(v.to_bits().to_le_bytes());
                    h.update(id.to_le_bytes());
                }
                None => h.update([2]),
            }
        }
        h.finalize()
            .iter()
            .take(12)
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Ranges of `piles` (sorted by tracker) that belong to one tracker.
    fn tracker_ranges(piles: &[MasterPile]) -> Vec<std::ops::Range<usize>> {
        let mut out = Vec::new();
        let mut start = 0;
        for i in 1..=piles.len() {
            if i == piles.len() || piles[i].tracker != piles[start].tracker {
                if start < i {
                    out.push(start..i);
                }
                start = i;
            }
        }
        out
    }

    /// Optimise every tracker whose inputs (elevations, settings) changed since
    /// its latest run, or every complete tracker when `force` is set. Trackers
    /// with a pile that has no elevation yet are left waiting.
    pub fn optimize(&mut self, force: bool, trigger: &str, threads: usize) -> Result<RunSummary> {
        let started = chrono::Utc::now();
        let settings = self.settings()?;
        let params = settings.params();
        let params_json = settings.params_json();
        let piles = self.master_piles()?;
        let elevs = self.latest_elevations()?;
        let floods = self.latest_flood()?;
        let latest = self.latest_runs()?;

        let mut summary = RunSummary {
            trigger: trigger.into(),
            ..Default::default()
        };
        let mut todo: Vec<(Tracker, String, Vec<Elev>)> = Vec::new();
        for range in Self::tracker_ranges(&piles) {
            let tp = &piles[range];
            let ev: Option<Vec<&Elev>> = tp.iter().map(|p| elevs.get(&p.pile_id)).collect();
            let Some(ev) = ev else {
                summary.awaiting += 1;
                continue;
            };
            let sig = Self::input_sig(&params_json, tp, &ev, &floods);
            if !force && latest.get(&tp[0].tracker).is_some_and(|l| l.sig == sig) {
                summary.up_to_date += 1;
                continue;
            }
            let tracker = Tracker {
                id: tp[0].tracker.clone(),
                piles: tp
                    .iter()
                    .zip(&ev)
                    .map(|(p, e)| Pile {
                        name: p.pile_id.clone(),
                        tracker: p.tracker.clone(),
                        block: p.block.clone(),
                        section: p.section.clone(),
                        pile_type: p.pile_type.clone(),
                        easting: p.easting,
                        northing: p.northing,
                        z_lidar: e.z,
                        min_top: settings.min_top(e.z, floods.get(&p.pile_id).map(|f| f.0)),
                    })
                    .collect(),
            };
            todo.push((tracker, sig, ev.into_iter().cloned().collect()));
        }

        if todo.is_empty() {
            summary.message = if summary.up_to_date > 0 {
                "Every tracker with complete elevations is already up to date.".into()
            } else {
                "No tracker has an elevation for every pile yet: nothing to optimise.".into()
            };
            summary.duration_ms = (chrono::Utc::now() - started).num_milliseconds().max(0) as u64;
            return Ok(summary);
        }

        let trackers: Vec<Tracker> = todo.iter().map(|(t, _, _)| t.clone()).collect();
        let results = optimize_all(&trackers, &params, &default_solver(), threads.max(1));
        let created_at = now();

        let tx = self.conn.transaction().map_err(db)?;
        tx.execute(
            "INSERT INTO runs (created_at, params, trigger, trackers_run, duration_ms) VALUES (?1, ?2, ?3, ?4, 0)",
            params![created_at, params_json, trigger, todo.len() as i64],
        )
        .map_err(db)?;
        let run_id = tx.last_insert_rowid();
        {
            let mut ins_t = tx
                .prepare(
                    "INSERT INTO run_trackers (run_id, tracker, status, input_sig, n_survey, n_lidar, max_angle_deg, max_cut_fill,
                                               max_shift, span_error, anchor_pile, anchor_motor)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                )
                .map_err(db)?;
            let mut ins_p = tx
                .prepare(
                    "INSERT INTO run_piles (run_id, pile_id, z_ground, z_import_id, new_ground, cut_fill, top, reveal, angle_diff_deg, outlier,
                                            min_top, min_top_active, set_e, set_n, shift)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                )
                .map_err(db)?;
            for ((tracker, sig, ev), res) in todo.iter().zip(&results) {
                let n_survey = ev.iter().filter(|e| e.kind == Kind::Survey).count();
                let status = res.status.as_str();
                *summary.counts.entry(status.to_string()).or_default() += 1;
                ins_t
                    .execute(params![
                        run_id,
                        tracker.id,
                        status,
                        sig,
                        n_survey as i64,
                        (ev.len() - n_survey) as i64,
                        res.max_angle_deg,
                        res.max_cut_fill,
                        res.anchor.map(|_| res.max_shift),
                        res.anchor.map(|_| res.max_span_error),
                        res.anchor.map(|(i, _)| tracker.piles[i].name.clone()),
                        res.anchor.map(|(_, motor)| motor as i64)
                    ])
                    .map_err(db)?;
                for ((p, e), r) in tracker.piles.iter().zip(ev).zip(&res.piles) {
                    ins_p
                        .execute(params![
                            run_id,
                            p.name,
                            e.z,
                            e.import_id,
                            r.new_ground,
                            r.cut_fill,
                            r.top,
                            r.final_reveal,
                            r.angle_diff_deg,
                            r.lidar_outlier as i64,
                            p.min_top,
                            r.min_top_active as i64,
                            r.easting,
                            r.northing,
                            r.shift
                        ])
                        .map_err(db)?;
                }
            }
        }
        tx.commit().map_err(db)?;

        summary.run_id = Some(run_id);
        summary.created_at = created_at;
        summary.trackers_run = todo.len();
        summary.duration_ms = (chrono::Utc::now() - started).num_milliseconds().max(0) as u64;
        summary.message = format!(
            "Optimised {} trackers in {} ms.",
            todo.len(),
            summary.duration_ms
        );
        Ok(summary)
    }

    /// Current state of every pile and tracker.
    pub fn state(&self) -> Result<State> {
        let settings = self.settings()?;
        let params_json = settings.params_json();
        let masters = self.master_piles()?;
        let elevs = self.latest_elevations()?;
        let floods = self.latest_flood()?;
        let runs = self.latest_runs()?;
        let mut results = self.latest_pile_results(&masters, &runs)?;
        let counts = self.survey_counts()?;
        let kinds = self.ground_by_kind()?;

        let mut piles = Vec::with_capacity(masters.len());
        let mut trackers = Vec::new();
        for range in Self::tracker_ranges(&masters) {
            let tp = &masters[range.clone()];
            let ev: Vec<Option<&Elev>> = tp.iter().map(|p| elevs.get(&p.pile_id)).collect();
            let n_survey = ev
                .iter()
                .flatten()
                .filter(|e| e.kind == Kind::Survey)
                .count();
            let n_lidar = ev
                .iter()
                .flatten()
                .filter(|e| e.kind == Kind::Lidar)
                .count();
            let n_missing = ev.iter().filter(|e| e.is_none()).count();
            let current_sig = (n_missing == 0).then(|| {
                Self::input_sig(
                    &params_json,
                    tp,
                    &ev.iter().map(|e| e.expect("complete")).collect::<Vec<_>>(),
                    &floods,
                )
            });
            let run = runs.get(&tp[0].tracker);
            let (status, stale) = match run {
                Some(r) => (
                    r.status.clone(),
                    current_sig.as_deref() != Some(r.sig.as_str()),
                ),
                None if n_missing > 0 => ("AwaitingSurvey".to_string(), false),
                None => ("NotRun".to_string(), true),
            };
            let n_flood = tp
                .iter()
                .filter(|p| floods.contains_key(&p.pile_id))
                .count();
            let n_flood_raised = tp
                .iter()
                .filter(|p| results.get(&p.pile_id).is_some_and(|r| r.min_top_active))
                .count();
            trackers.push(TrackerState {
                tracker: tp[0].tracker.clone(),
                block: tp[0].block.clone(),
                section: tp[0].section.clone(),
                tracker_type: tp[0].tracker_type.clone(),
                n_piles: tp.len(),
                n_survey,
                n_lidar,
                n_missing,
                status,
                stale,
                run_id: run.map(|r| r.run_id),
                run_at: run.map(|r| r.run_at.clone()),
                max_angle_deg: run.map(|r| r.max_angle),
                max_cut_fill: run.map(|r| r.max_cut_fill),
                max_shift: run.and_then(|r| r.max_shift),
                span_error: run.and_then(|r| r.span_error),
                anchor_pile: run.and_then(|r| r.anchor_pile.clone()),
                anchor_motor: run.and_then(|r| r.anchor_motor),
                n_flood,
                n_flood_raised,
                first_pile: range.start,
            });
            for (p, e) in tp.iter().zip(ev) {
                let flood = floods.get(&p.pile_id);
                let flood_value = flood.map(|f| f.0);
                piles.push(PileState {
                    pile_id: p.pile_id.clone(),
                    tracker: p.tracker.clone(),
                    block: p.block.clone(),
                    section: p.section.clone(),
                    easting: p.easting,
                    northing: p.northing,
                    pile_type: p.pile_type.clone(),
                    color: p.color.clone(),
                    tracker_type: p.tracker_type.clone(),
                    z: e.map(|e| e.z),
                    z_source: e.map(|e| e.kind),
                    z_import_id: e.map(|e| e.import_id),
                    z_date: e.map(|e| e.date.clone()),
                    z_survey: kinds
                        .get(&p.pile_id)
                        .and_then(|g| g.survey.as_ref().map(|s| s.0)),
                    z_survey_date: kinds
                        .get(&p.pile_id)
                        .and_then(|g| g.survey.as_ref().map(|s| s.1.clone())),
                    z_lidar: kinds.get(&p.pile_id).and_then(|g| g.lidar),
                    offset_m: e.and_then(|e| e.offset_m),
                    n_surveys: counts.get(&p.pile_id).copied().unwrap_or(0),
                    flood: flood_value,
                    flood_import_id: flood.map(|f| f.1),
                    flood_level: e.and_then(|e| settings.flood_water_level(e.z, flood_value)),
                    min_top: e.and_then(|e| settings.min_top(e.z, flood_value)),
                    result: results.remove(&p.pile_id),
                });
            }
        }
        Ok(State { piles, trackers })
    }

    fn last_run(&self) -> Result<Option<RunSummary>> {
        let run: Option<(i64, String, String, i64, i64)> = self
            .conn
            .query_row(
                "SELECT run_id, created_at, trigger, trackers_run, duration_ms FROM runs ORDER BY run_id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(db)?;
        let Some((run_id, created_at, trigger, n, _)) = run else {
            return Ok(None);
        };
        let mut st = self
            .conn
            .prepare("SELECT status, COUNT(*) FROM run_trackers WHERE run_id = ?1 GROUP BY status")
            .map_err(db)?;
        let counts = st
            .query_map([run_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
            })
            .map_err(db)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db)?;
        Ok(Some(RunSummary {
            run_id: Some(run_id),
            created_at,
            trigger,
            trackers_run: n as usize,
            counts,
            ..Default::default()
        }))
    }

    pub fn summary(&self) -> Result<Summary> {
        self.summary_of(&self.state()?)
    }

    /// The summary of a state already read.
    pub(crate) fn summary_of(&self, state: &State) -> Result<Summary> {
        let mut tracker_status: BTreeMap<String, usize> = BTreeMap::new();
        let mut blocks: BTreeMap<String, BlockSummary> = BTreeMap::new();
        for t in &state.trackers {
            *tracker_status.entry(t.status.clone()).or_default() += 1;
            let b = blocks
                .entry(t.block.clone())
                .or_insert_with(|| BlockSummary {
                    block: t.block.clone(),
                    piles: 0,
                    surveyed: 0,
                    trackers: 0,
                    counts: BTreeMap::new(),
                    reveal_mean: None,
                });
            b.trackers += 1;
            b.piles += t.n_piles;
            b.surveyed += t.n_survey;
            *b.counts.entry(t.status.clone()).or_default() += 1;
        }
        let last_survey = self
            .conn
            .query_row(
                &format!("SELECT {} FROM imports WHERE kind = 'survey' AND voided_at IS NULL ORDER BY import_id DESC LIMIT 1", Self::IMPORT_COLS),
                [],
                |r| Self::import_row(r, false),
            )
            .optional()
            .map_err(db)?;
        let imports: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))
            .map_err(db)?;
        let mut block_reveals: HashMap<&str, Vec<f64>> = HashMap::new();
        for p in &state.piles {
            if let Some(r) = &p.result {
                block_reveals.entry(&p.block).or_default().push(r.reveal);
            }
        }
        let mut blocks: Vec<BlockSummary> = blocks.into_values().collect();
        for b in &mut blocks {
            b.reveal_mean = block_reveals
                .get(b.block.as_str())
                .and_then(|v| crate::blocks::mean(v));
        }
        blocks.sort_by_key(|a| natural_key(&a.block));
        let reveals: Vec<f64> = state
            .piles
            .iter()
            .filter_map(|p| p.result.as_ref().map(|r| r.reveal))
            .collect();
        let reveals_survey: Vec<f64> = state
            .piles
            .iter()
            .filter(|p| p.z_source == Some(Kind::Survey))
            .filter_map(|p| p.result.as_ref().map(|r| r.reveal))
            .collect();
        Ok(Summary {
            db_path: self.path.display().to_string(),
            piles: state.piles.len(),
            trackers: state.trackers.len(),
            piles_surveyed: state
                .piles
                .iter()
                .filter(|p| p.z_source == Some(Kind::Survey))
                .count(),
            piles_lidar_only: state
                .piles
                .iter()
                .filter(|p| p.z_source == Some(Kind::Lidar))
                .count(),
            piles_no_elevation: state.piles.iter().filter(|p| p.z.is_none()).count(),
            piles_with_flood: state.piles.iter().filter(|p| p.flood.is_some()).count(),
            piles_flood_raised: state
                .piles
                .iter()
                .filter(|p| p.result.as_ref().is_some_and(|r| r.min_top_active))
                .count(),
            piles_with_result: reveals.len(),
            reveal_mean: crate::blocks::mean(&reveals),
            reveal_mean_survey: crate::blocks::mean(&reveals_survey),
            reveal_min: reveals.iter().copied().reduce(f64::min),
            reveal_max: reveals.iter().copied().reduce(f64::max),
            stale_trackers: state.trackers.iter().filter(|t| t.stale).count(),
            tracker_status,
            imports: imports as usize,
            last_survey,
            last_run: self.last_run()?,
            blocks,
        })
    }
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2])
}

/// Sort key that orders "2" before "10".
pub fn natural_key(s: &str) -> (u64, String) {
    (s.parse::<u64>().unwrap_or(u64::MAX), s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export;

    const SAMPLE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../testdata/sample_block01_synthetic.csv"
    ));

    fn loaded() -> Store {
        let mut s = Store::open_in_memory().unwrap();
        let rep = s
            .import_file(Kind::Master, "piles.csv", SAMPLE, None, "test")
            .unwrap();
        assert!(rep.accepted, "{:?}", rep.findings);
        s
    }

    fn findings(rep: &ImportReport) -> String {
        rep.findings
            .iter()
            .map(|f| {
                format!(
                    "[{:?} x{}] {} {:?}",
                    f.level, f.count, f.message, f.examples
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn master_import_loads_piles_and_lidar() {
        let s = loaded();
        assert_eq!(s.pile_count().unwrap(), 789);
        let st = s.state().unwrap();
        assert_eq!(st.trackers.len(), 62);
        assert!(st.piles.iter().all(|p| p.z_source == Some(Kind::Lidar)));
        assert!(st.trackers.iter().all(|t| t.status == "NotRun" && t.stale));
        // Piles of a tracker come out ordered by northing.
        let t = &st.trackers[0];
        let ns: Vec<f64> = st.piles[t.first_pile..t.first_pile + t.n_piles]
            .iter()
            .map(|p| p.northing)
            .collect();
        assert!(ns.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn master_list_is_immutable() {
        let mut s = loaded();
        let err = s
            .conn
            .execute(
                "UPDATE piles SET easting = 0 WHERE pile_id = 'P01.02.01.01'",
                [],
            )
            .unwrap_err();
        assert!(db(err).contains("immutable"));
        let err = s.conn.execute("DELETE FROM piles", []).unwrap_err();
        assert!(db(err).contains("immutable"));
        assert!(s.conn.execute("DELETE FROM imports", []).is_err());
        assert!(
            s.conn
                .execute(
                    "UPDATE imports SET voided_at = 'x' WHERE kind = 'master'",
                    []
                )
                .is_err()
        );
        assert!(s.void_import(1, "test").is_err());

        // Same file again: refused as a duplicate.
        let rep = s
            .import_file(Kind::Master, "piles.csv", SAMPLE, None, "test")
            .unwrap();
        assert!(!rep.accepted);
        assert!(findings(&rep).contains("already imported"));

        // Changed coordinates for an existing pile: refused, nothing written.
        let changed =
            "Pile Name,Tracker,Easting,Northing\nP01.02.01.01,P01.02.01,1.0,2.0\nNEW.1,NEW,5,5\n";
        let rep = s
            .import_file(Kind::Master, "v2.csv", changed.as_bytes(), None, "test")
            .unwrap();
        assert!(!rep.accepted, "{}", findings(&rep));
        assert_eq!(s.pile_count().unwrap(), 789);

        // Only new piles: added.
        let added = "Pile Name,Easting,Northing,Color\nNEW.01.1,5,5,Red\nNEW.01.2,5,13,Blue\n";
        let rep = s
            .import_file(Kind::Master, "v3.csv", added.as_bytes(), None, "test")
            .unwrap();
        assert!(rep.accepted, "{}", findings(&rep));
        assert_eq!(s.pile_count().unwrap(), 791);
        let st = s.state().unwrap();
        let p = st.piles.iter().find(|p| p.pile_id == "NEW.01.2").unwrap();
        assert_eq!((p.tracker.as_str(), p.color.as_str()), ("NEW.01", "Blue"));
    }

    #[test]
    fn master_list_with_bad_rows_is_refused_whole() {
        let mut s = Store::open_in_memory().unwrap();
        let bad = "Pile Name,Easting,Northing\nA.1,1,2\nA.1,1,3\nA.2,x,3\n";
        let rep = s
            .import_file(Kind::Master, "bad.csv", bad.as_bytes(), None, "test")
            .unwrap();
        assert!(!rep.accepted);
        assert_eq!(s.pile_count().unwrap(), 0);
        let text = findings(&rep);
        assert!(
            text.contains("more than once") && text.contains("not a number"),
            "{text}"
        );
    }

    #[test]
    fn survey_needs_master_first() {
        let mut s = Store::open_in_memory().unwrap();
        let rep = s
            .import_file(Kind::Survey, "s.csv", b"P1,1,2,3\n", None, "test")
            .unwrap();
        assert!(!rep.accepted);
        assert!(findings(&rep).contains("master pile list first"));
    }

    #[test]
    fn survey_import_matching_rules_and_incremental_runs() {
        let mut s = loaded();
        let first = s.optimize(false, "test", 2).unwrap();
        assert_eq!(first.trackers_run, 62);
        let again = s.optimize(false, "test", 2).unwrap();
        assert_eq!((again.trackers_run, again.up_to_date), (0, 62));
        assert!(again.run_id.is_none());

        // Daily survey, semicolon CSV with decimal commas, as a French export would be.
        // P01.02.01.01..03: by ID, ground 2 cm lower than LIDAR.
        // 'GCP-7' with no ID match but 0.12 m from P01.02.01.04: matched by position (and offset warning).
        // P01.02.01.05 with coordinates 3 m off: rejected (wrong ID).
        // 'BASE' far from any pile: skipped.
        let survey = "Point;Est;Nord;Altitude;Code\n\
            P01.02.01.01;305512,5968;6428751,2501;49,461;TN\n\
            p01.02.01.02;305512,6000;6428743,3355;49,862;TN\n\
            P01.02.01.03;305512,5968;6428735,0761;50,297;TN\n\
            GCP-7;305512,7168;6428726,8167;50,656;TN\n\
            P01.02.01.05;305515,5968;6428719,7133;51,031;TN\n\
            BASE;305000;6428000;40;STN\n";
        let rep = s
            .import_file(
                Kind::Survey,
                "Leve pieux 03.11.2026.csv",
                survey.as_bytes(),
                None,
                "test",
            )
            .unwrap();
        let text = findings(&rep);
        assert!(rep.accepted, "{text}");
        assert_eq!(
            (rep.rows_total, rep.rows_used, rep.rows_rejected),
            (6, 4, 2),
            "{text}"
        );
        assert_eq!(rep.survey_date.as_deref(), Some("2026-11-03"));
        assert_eq!(rep.survey_date_source.as_deref(), Some("file name"));
        assert!(text.contains("matched to the nearest pile"), "{text}");
        assert!(text.contains("Rejected: more than 1 m"), "{text}");
        assert!(text.contains("no pile within"), "{text}");
        assert!(
            text.contains("more than 0.1 m from the pile's design position"),
            "{text}"
        );

        let st = s.state().unwrap();
        let t = st
            .trackers
            .iter()
            .find(|t| t.tracker == "P01.02.01")
            .unwrap();
        assert_eq!((t.n_survey, t.n_lidar, t.n_missing), (4, 1, 0));
        assert!(t.stale);
        assert_eq!(st.trackers.iter().filter(|t| t.stale).count(), 1);
        let p4 = st
            .piles
            .iter()
            .find(|p| p.pile_id == "P01.02.01.04")
            .unwrap();
        assert_eq!(p4.z, Some(50.656));
        assert_eq!(p4.z_source, Some(Kind::Survey));
        assert!((p4.offset_m.unwrap() - 0.12).abs() < 1e-6);

        // Only the tracker with new elevations is re-optimised, using them.
        let run = s.optimize(false, "test", 2).unwrap();
        assert_eq!(run.trackers_run, 1);
        let st = s.state().unwrap();
        assert!(st.trackers.iter().all(|t| !t.stale));
        let p1 = st
            .piles
            .iter()
            .find(|p| p.pile_id == "P01.02.01.01")
            .unwrap();
        let r = p1.result.as_ref().unwrap();
        assert_eq!(r.z_ground, 49.461);
        assert!((r.top - r.new_ground - r.reveal).abs() < 1e-9);

        // Same file twice: refused.
        let dup = s
            .import_file(Kind::Survey, "copy.csv", survey.as_bytes(), None, "test")
            .unwrap();
        assert!(!dup.accepted);

        // A later survey with a big jump is kept but flagged.
        let later = "Pile ID,Easting,Northing,Z,Date\nP01.02.01.01,305512.5968,6428751.2501,49.961,04/11/2026\n";
        let rep2 = s
            .import_file(Kind::Survey, "s2.csv", later.as_bytes(), None, "test")
            .unwrap();
        assert!(
            findings(&rep2).contains("changed by more than 0.3 m"),
            "{}",
            findings(&rep2)
        );
        assert_eq!(rep2.survey_date.as_deref(), Some("2026-11-04"));
        assert_eq!(rep2.survey_date_source.as_deref(), Some("date column"));
        let st = s.state().unwrap();
        let p1 = st
            .piles
            .iter()
            .find(|p| p.pile_id == "P01.02.01.01")
            .unwrap();
        assert_eq!((p1.z, p1.n_surveys), (Some(49.961), 2));

        // Voiding it brings back the previous survey value; the tracker needs a re-run.
        s.void_import(rep2.import_id.unwrap(), "rod height error")
            .unwrap();
        assert!(s.void_import(rep2.import_id.unwrap(), "again").is_err());
        let st = s.state().unwrap();
        let p1 = st
            .piles
            .iter()
            .find(|p| p.pile_id == "P01.02.01.01")
            .unwrap();
        assert_eq!((p1.z, p1.n_surveys), (Some(49.461), 1));
        // The second survey was never optimised, and the latest run used 49.461,
        // which is the value in force again: nothing is stale.
        assert_eq!(st.trackers.iter().filter(|t| t.stale).count(), 0);
        // The voided file can be imported again.
        let again = s
            .import_file(Kind::Survey, "s2.csv", later.as_bytes(), None, "test")
            .unwrap();
        assert!(again.accepted);
    }

    fn pile<'a>(st: &'a State, id: &str) -> &'a PileState {
        st.piles.iter().find(|p| p.pile_id == id).unwrap()
    }

    #[test]
    fn flood_layer_raises_tops_and_follows_settings() {
        let mut s = Store::open_in_memory().unwrap();
        // Like the project master file: design ground, raw LIDAR, tracker type, empty flood column.
        let mut master = String::from(
            "Pile Name;Tracker;Tracker type;Pile Type;Easting;Northing;Z LIDAR_CF;Z LIDAR;Flood;Color\n",
        );
        for t in 1..=2 {
            for i in 0..5 {
                master.push_str(&format!(
                    "P{t}.{i};T{t};28 MODULE EXT TRACKER;SAP-7;{};{};10,0;10,3;;RED\n",
                    t * 10,
                    i * 8
                ));
            }
        }
        let rep = s
            .import_file(Kind::Master, "master.csv", master.as_bytes(), None, "t")
            .unwrap();
        let text = findings(&rep);
        assert!(rep.accepted, "{text}");
        assert!(
            text.contains("read from \"Z LIDAR_CF\"; also in the file but not used: Z LIDAR"),
            "{text}"
        );
        assert!(text.contains("flood column (\"Flood\") is empty"), "{text}");
        let st = s.state().unwrap();
        assert_eq!(pile(&st, "P1.0").z, Some(10.0));
        assert_eq!(pile(&st, "P1.0").tracker_type, "28 MODULE EXT TRACKER");
        assert_eq!(st.trackers[0].tracker_type, "28 MODULE EXT TRACKER");
        s.optimize(false, "t", 1).unwrap();
        assert!(
            s.state()
                .unwrap()
                .piles
                .iter()
                .all(|p| (p.result.as_ref().unwrap().top - 11.45).abs() < 1e-4)
        );

        // The core cannot change, tracker type included.
        let changed = master.replace("28 MODULE EXT TRACKER", "56 MODULE EXT TRACKER");
        assert!(
            !s.import_file(Kind::Master, "m2.csv", changed.as_bytes(), None, "t")
                .unwrap()
                .accepted
        );

        // This test reads depths, with the top of pile right at the water.
        let mut set = s.settings().unwrap();
        set.flood_mode = crate::FloodMode::Depth;
        (
            set.flood_pile_height_m,
            set.flood_module_clearance_m,
            set.flood_freeboard_m,
        ) = (0.0, 0.0, 0.0);
        s.set_settings(&set).unwrap();
        s.optimize(false, "t", 1).unwrap();
        // Flood depths for tracker T1 only: 1.50 m of water, above the 1.45 m nominal reveal.
        let flood = "Pile ID,Flood depth (m)\nP1.0,1.5\nP1.1,1.5\nP1.2,1.5\nP1.3,1.5\nP1.4,1.5\n";
        let rep = s
            .import_file(Kind::Flood, "flood.csv", flood.as_bytes(), None, "t")
            .unwrap();
        assert!(rep.accepted, "{}", findings(&rep));
        assert_eq!(
            rep.columns.get("Flood").map(String::as_str),
            Some("Flood depth (m)")
        );
        let st = s.state().unwrap();
        assert_eq!((st.trackers[0].stale, st.trackers[1].stale), (true, false));
        assert_eq!(pile(&st, "P1.2").flood_level, Some(11.5));
        assert_eq!(pile(&st, "P1.2").min_top, Some(11.5));
        assert_eq!(s.optimize(false, "t", 1).unwrap().trackers_run, 1);
        let st = s.state().unwrap();
        let r = pile(&st, "P1.2").result.clone().unwrap();
        assert!(
            r.top >= 11.5 - 1e-4 && r.min_top_active && r.min_top == Some(11.5),
            "{r:?}"
        );
        assert_eq!(
            (st.trackers[0].n_flood, st.trackers[0].n_flood_raised),
            (5, 5)
        );
        assert_eq!(st.trackers[0].status, "Compliant");
        let sum = s.summary().unwrap();
        assert_eq!((sum.piles_with_flood, sum.piles_flood_raised), (5, 5));

        // 0.25 m clearance: tops must be 1.75 m above ground, beyond the 1.60 m
        // maximum reveal, so about 0.15 m of fill is needed.
        let mut set = s.settings().unwrap();
        set.flood_freeboard_m = 0.25;
        s.set_settings(&set).unwrap();
        s.optimize(false, "t", 1).unwrap();
        let st = s.state().unwrap();
        let r = pile(&st, "P1.2").result.clone().unwrap();
        assert!(
            r.top >= 11.75 - 1e-4 && (r.cut_fill - 0.15).abs() < 2e-3,
            "{r:?}"
        );
        assert_eq!(st.trackers[0].status, "Graded");
        assert_eq!(st.trackers[1].status, "Compliant");

        // Same file again: refused. A newer file (water elevations) replaces it.
        assert!(
            !s.import_file(Kind::Flood, "again.csv", flood.as_bytes(), None, "t")
                .unwrap()
                .accepted
        );
        set.flood_mode = crate::FloodMode::Elevation;
        set.flood_freeboard_m = 0.0;
        s.set_settings(&set).unwrap();
        let levels = "Pile ID;PHE\nP1.0;11,0\nP1.1;11,0\nP1.2;11,62\nP1.3;11,0\nP1.4;11,0\n";
        let rep = s
            .import_file(Kind::Flood, "phe.csv", levels.as_bytes(), None, "t")
            .unwrap();
        assert!(rep.accepted, "{}", findings(&rep));
        s.optimize(false, "t", 1).unwrap();
        let st = s.state().unwrap();
        assert_eq!(pile(&st, "P1.0").min_top, Some(11.0));
        assert!(!pile(&st, "P1.0").result.as_ref().unwrap().min_top_active);
        let r = pile(&st, "P1.2").result.clone().unwrap();
        assert!(r.top >= 11.62 - 1e-4 && r.min_top_active, "{r:?}");

        // Voiding the flood imports removes the requirement.
        s.void_import(rep.import_id.unwrap(), "wrong datum")
            .unwrap();
        let id = s
            .imports()
            .unwrap()
            .iter()
            .find(|i| i.file_name == "flood.csv")
            .unwrap()
            .import_id;
        s.void_import(id, "superseded").unwrap();
        s.optimize(false, "t", 1).unwrap();
        let st = s.state().unwrap();
        assert!(
            st.piles
                .iter()
                .all(|p| p.flood.is_none() && !p.result.as_ref().unwrap().min_top_active)
        );
        assert!((pile(&st, "P1.2").result.as_ref().unwrap().top - 11.45).abs() < 1e-4);
    }

    #[test]
    fn master_with_flood_values_records_them() {
        let mut s = Store::open_in_memory().unwrap();
        let master =
            "Pile ID,Easting,Northing,Z,Flood\nA.1,0,0,5,0.2\nA.2,0,8,5,\nA.3,0,16,5,0.4\n";
        let rep = s
            .import_file(Kind::Master, "m.csv", master.as_bytes(), None, "t")
            .unwrap();
        assert!(
            findings(&rep).contains("2 flood values were also recorded"),
            "{}",
            findings(&rep)
        );
        let st = s.state().unwrap();
        assert_eq!(
            st.piles.iter().map(|p| p.flood).collect::<Vec<_>>(),
            vec![Some(0.2), None, Some(0.4)]
        );
        assert!(
            s.imports()
                .unwrap()
                .iter()
                .any(|i| i.kind == "flood" && i.rows_used == 2)
        );
    }

    #[test]
    fn preview_database_is_refused_with_a_clear_message() {
        let path = std::env::temp_dir().join(format!("tracker-top-v1-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); INSERT INTO meta VALUES ('schema_version', '1');").unwrap();
        }
        let err = Store::open(&path).err().unwrap();
        assert!(err.contains("preview version"), "{err}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn survey_columns_recognised_from_values() {
        let mut s = loaded();
        // Header row, point numbers first, pile ID in a later "Description" column
        // with a suffix, Northing before Easting.
        let a = "Point;Nord;Est;Z;Description\n\
            1001;6428751,2501;305512,5968;49,461;P01.02.01.01_TN\n\
            1002;6428743,3355;305512,5968;49,862;P01.02.01.02_TN\n\
            1003;6428735,0761;305512,5968;50,297;TN P01.02.01.03\n";
        let rep = s
            .import_file(Kind::Survey, "a.csv", a.as_bytes(), None, "t")
            .unwrap();
        let text = findings(&rep);
        assert!(rep.accepted, "{text}");
        assert_eq!(rep.rows_used, 3, "{text}");
        assert!(
            text.contains("Pile IDs recognised in column \"Description\""),
            "{text}"
        );
        assert!(!text.contains("matched to the nearest pile"), "{text}");
        assert_eq!(rep.columns.get("Easting").map(String::as_str), Some("Est"));

        // No header: point number, Northing, Easting, Elevation, pile ID (PNEZD).
        let b = "2001,6428726.8167,305512.5968,50.656,P01.02.01.04\n\
            2002,6428719.7133,305512.5968,51.031,P01.02.01.05\n";
        let rep = s
            .import_file(Kind::Survey, "b.txt", b.as_bytes(), None, "t")
            .unwrap();
        let text = findings(&rep);
        assert!(rep.accepted, "{text}");
        assert_eq!(rep.rows_used, 2, "{text}");
        assert_eq!(
            rep.columns.get("Pile ID").map(String::as_str),
            Some("Column 5")
        );
        assert_eq!(
            rep.columns.get("Easting").map(String::as_str),
            Some("Column 3")
        );
        assert_eq!(
            rep.columns.get("Northing").map(String::as_str),
            Some("Column 2")
        );
        assert_eq!(
            rep.columns.get("Elevation").map(String::as_str),
            Some("Column 4")
        );
        let st = s.state().unwrap();
        let p = st
            .piles
            .iter()
            .find(|p| p.pile_id == "P01.02.01.05")
            .unwrap();
        assert_eq!(p.z, Some(51.031));
        assert!(p.offset_m.unwrap() < 1e-6);

        // X / Y headers the other way round: X holds northings.
        let c = "ID,X,Y,Z\nP01.02.01.06,6428711.4539,305512.5968,51.4\n";
        let rep = s
            .import_file(Kind::Survey, "c.csv", c.as_bytes(), None, "t")
            .unwrap();
        let text = findings(&rep);
        assert!(text.contains("swapped"), "{text}");
        assert_eq!(rep.columns.get("Easting").map(String::as_str), Some("Y"));
        assert!(!text.contains("Rejected"), "{text}");
    }

    #[test]
    fn headerless_survey_without_reference_elevations() {
        let mut s = Store::open_in_memory().unwrap();
        let master = "Pile ID,Easting,Northing\nA.1,500000,6000000\nA.2,500000,6000008\nA.3,500000,6000016\n";
        assert!(
            s.import_file(Kind::Master, "m.csv", master.as_bytes(), None, "t")
                .unwrap()
                .accepted
        );
        // Point number, Easting, Northing, Elevation, Code: elevation is the
        // first numeric column after the coordinates.
        let survey = "7,500000.01,6000000.00,120.50,A.1\n8,500000.00,6000008.02,120.61,A.2\n";
        let rep = s
            .import_file(Kind::Survey, "s.txt", survey.as_bytes(), None, "t")
            .unwrap();
        assert!(rep.accepted, "{}", findings(&rep));
        assert_eq!(
            rep.columns.get("Elevation").map(String::as_str),
            Some("Column 4")
        );
        assert_eq!(
            rep.columns.get("Pile ID").map(String::as_str),
            Some("Column 5")
        );
    }

    #[test]
    fn lidar_file_is_not_taken_as_a_survey() {
        let mut s = loaded();
        let rep = s
            .import_file(Kind::Survey, "piles.csv", SAMPLE, None, "test")
            .unwrap();
        assert!(!rep.accepted);
        assert!(findings(&rep).contains("looks like LIDAR"));
        s.set_watch_folder(Some("C:/Sync/Survey")).unwrap();
        assert_eq!(s.watch_folder().unwrap().as_deref(), Some("C:/Sync/Survey"));
        s.set_watch_folder(None).unwrap();
        assert_eq!(s.watch_folder().unwrap(), None);
    }

    #[test]
    fn settings_change_reruns_everything() {
        let mut s = loaded();
        s.optimize(false, "test", 2).unwrap();
        let mut set = s.settings().unwrap();
        set.max_angle_deg = 0.6;
        s.set_settings(&set).unwrap();
        assert_eq!(
            s.state()
                .unwrap()
                .trackers
                .iter()
                .filter(|t| t.stale)
                .count(),
            62
        );
        assert_eq!(s.optimize(false, "test", 2).unwrap().trackers_run, 62);
        // Matching tolerances do not affect results.
        set.match_radius_m = 0.8;
        s.set_settings(&set).unwrap();
        assert_eq!(
            s.state()
                .unwrap()
                .trackers
                .iter()
                .filter(|t| t.stale)
                .count(),
            0
        );
        set.offset_warn_m = 5.0;
        assert!(s.set_settings(&set).is_err());
    }

    #[test]
    fn trackers_wait_for_every_pile() {
        let mut s = Store::open_in_memory().unwrap();
        let master = "Pile ID,Easting,Northing,Pile Type\nT1.1,0,0,A\nT1.2,0,8,A\nT1.3,0,16,A\nT2.1,10,0,A\nT2.2,10,8,A\nT2.3,10,16,A\n";
        assert!(
            s.import_file(Kind::Master, "m.csv", master.as_bytes(), None, "t")
                .unwrap()
                .accepted
        );
        let survey = "T1.1,0,0,10.0\nT1.2,0,8,10.1\nT1.3,0,16,10.2\nT2.1,10,0,10.0\n";
        let rep = s
            .import_file(
                Kind::Survey,
                "s.txt",
                survey.as_bytes(),
                Some("2026-11-02"),
                "t",
            )
            .unwrap();
        assert!(rep.accepted, "{}", findings(&rep));
        assert_eq!(rep.survey_date_source.as_deref(), Some("entered"));
        let run = s.optimize(false, "t", 1).unwrap();
        assert_eq!((run.trackers_run, run.awaiting), (1, 1));
        let st = s.state().unwrap();
        assert_eq!(st.trackers[0].status, "Compliant");
        assert_eq!(
            (st.trackers[1].status.as_str(), st.trackers[1].n_missing),
            ("AwaitingSurvey", 2)
        );
    }

    #[test]
    fn exports_contain_every_pile() {
        let mut s = loaded();
        s.optimize(false, "test", 2).unwrap();
        let st = s.state().unwrap();
        let csv = export::piles_csv(&st, &s.settings().unwrap());
        assert_eq!(csv.lines().count(), 790);
        assert!(csv.starts_with("Pile ID,Block,Section,Tracker"));
        let xlsx = export::workbook(&st, &s.imports().unwrap(), &s.settings().unwrap()).unwrap();
        assert!(xlsx.starts_with(b"PK"));
        // Round trip through the spreadsheet reader.
        let t = read_table("x.xlsx", &xlsx).unwrap();
        assert_eq!(t.rows.len(), 789);
        assert_eq!(t.find(Col::PileId), Some(0));
    }

    #[test]
    fn geopackage_export() {
        let mut s = loaded();
        s.optimize(false, "test", 2).unwrap();
        let mut set = s.settings().unwrap();
        set.epsg = Some(7856);
        s.set_settings(&set).unwrap();
        let bytes = crate::gpkg::geopackage(&s.state().unwrap(), &s.settings().unwrap()).unwrap();
        assert_eq!(&bytes[..16], b"SQLite format 3\0");
        assert_eq!(&bytes[68..72], &0x4750_4B47u32.to_be_bytes()); // application_id 'GPKG'
        let path = std::env::temp_dir().join(format!("tt-gpkg-test-{}.gpkg", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let c = Connection::open(&path).unwrap();
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM piles", [], |r| r.get(0))
            .unwrap();
        let t: i64 = c
            .query_row("SELECT COUNT(*) FROM trackers", [], |r| r.get(0))
            .unwrap();
        assert_eq!((n, t), (789, 62));
        let (srs, wkt): (i64, String) = c
            .query_row("SELECT g.srs_id, s.definition FROM gpkg_geometry_columns g JOIN gpkg_spatial_ref_sys s USING (srs_id) WHERE g.table_name = 'piles'", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(srs, 7856);
        assert!(
            wkt.starts_with("PROJCS[\"GDA2020 / MGA zone 56\"")
                && wkt.contains("\"central_meridian\",153")
        );
        let (geom, top): (Vec<u8>, f64) = c
            .query_row(
                "SELECT geom, top FROM piles WHERE pile_id = 'P01.02.01.01'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(&geom[..4], &[b'G', b'P', 0, 1]);
        assert_eq!(i32::from_le_bytes(geom[4..8].try_into().unwrap()), 7856);
        assert_eq!(u32::from_le_bytes(geom[9..13].try_into().unwrap()), 1001);
        assert_eq!(f64::from_le_bytes(geom[29..37].try_into().unwrap()), top);
        drop(c);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn set_out_keeps_spans_and_anchors_motor_piles() {
        let mut s = loaded();
        s.optimize(false, "test", 2).unwrap();
        let st = s.state().unwrap();
        let mut max_shift = 0.0f64;
        for t in &st.trackers {
            assert_eq!(t.anchor_motor, Some(true), "{}", t.tracker);
            // Set-out positions are rounded to the centimetre: each span's
            // length is then right to about 1 cm.
            assert!(
                t.span_error.unwrap() <= 0.0101,
                "{}: {:?}",
                t.tracker,
                t.span_error
            );
            let anchor = st
                .piles
                .iter()
                .find(|p| Some(&p.pile_id) == t.anchor_pile.as_ref())
                .unwrap();
            assert!(anchor.result.as_ref().unwrap().shift.unwrap().abs() < 1e-12);
            max_shift = max_shift.max(t.max_shift.unwrap());
        }
        assert!(max_shift > 0.005, "{max_shift}");
        let p = st
            .piles
            .iter()
            .find(|p| p.pile_id == "P01.02.01.01")
            .unwrap();
        let r = p.result.as_ref().unwrap();
        // Set-out positions and shifts are rounded to the centimetre.
        assert!(((r.set_northing.unwrap() - p.northing) - r.shift.unwrap()).abs() < 0.0101);
        assert!((r.set_easting.unwrap() - p.easting).abs() <= 0.005 + 1e-9);
        let csv = export::piles_csv(&st, &s.settings().unwrap());
        assert!(
            csv.lines()
                .next()
                .unwrap()
                .contains("Set-out Easting,Set-out Northing,Shift Along Tracker (m)")
        );

        // Turned off: design positions, everything re-optimised.
        let mut set = s.settings().unwrap();
        set.fixed_spans = false;
        s.set_settings(&set).unwrap();
        assert_eq!(s.optimize(false, "test", 2).unwrap().trackers_run, 62);
        let st = s.state().unwrap();
        assert!(
            st.piles
                .iter()
                .all(|p| p.result.as_ref().unwrap().shift == Some(0.0))
        );
    }

    #[test]
    fn version_2_database_is_upgraded_in_place() {
        let path = std::env::temp_dir().join(format!("tracker-top-v2-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            // The version 2 schema: today's without the set-out columns.
            // (Git may check the file out with Windows line endings.)
            let v2 = SCHEMA
                .replace("\r\n", "\n")
                .replace("    max_shift     REAL,\n    span_error    REAL,\n    anchor_pile   TEXT,\n    anchor_motor  INTEGER,\n", "")
                .replace("    set_e          REAL,\n    set_n          REAL,\n    shift          REAL,\n", "");
            assert!(!v2.contains("set_e") && !v2.contains("anchor_pile"));
            let c = Connection::open(&path).unwrap();
            c.execute_batch(&v2).unwrap();
            c.execute_batch(
                "INSERT INTO meta VALUES ('schema_version', '2');
                 INSERT INTO imports (kind, file_name, sha256, imported_at, origin, rows_total, rows_used, rows_rejected, report)
                   VALUES ('master', 'm.csv', 'x', '2026-11-04', 'test', 1, 1, 0, '{}');
                 INSERT INTO piles (pile_id, tracker, easting, northing, import_id) VALUES ('A.1', 'A', 0, 0, 1);",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.pile_count().unwrap(), 1);
        assert_eq!(s.settings().unwrap().epsg, Some(7856));
        assert_eq!(
            s.meta("schema_version").unwrap().as_deref(),
            Some(SCHEMA_VERSION)
        );
        drop(s);
        // Opening again is a no-op.
        assert!(Store::open(&path).is_ok());
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
    }

    #[test]
    fn summary_counts() {
        let mut s = loaded();
        s.optimize(false, "test", 2).unwrap();
        let sum = s.summary().unwrap();
        assert_eq!(
            (
                sum.piles,
                sum.trackers,
                sum.piles_lidar_only,
                sum.piles_surveyed
            ),
            (789, 62, 789, 0)
        );
        assert_eq!(sum.tracker_status.values().sum::<usize>(), 62);
        assert_eq!(sum.blocks.len(), 1);
        assert_eq!(sum.imports, 2);
    }

    #[test]
    fn survey_and_lidar_ground_are_both_kept() {
        let mut s = loaded();
        let st = s.state().unwrap();
        let p0 = st.piles[0].clone();
        let lidar = p0.z_lidar.unwrap();
        assert_eq!((p0.z, p0.z_survey), (Some(lidar), None));
        let survey = format!(
            "{},{},{},{:.3}\n",
            p0.pile_id,
            p0.easting,
            p0.northing,
            lidar + 0.123
        );
        let rep = s
            .import_file(
                Kind::Survey,
                "s.csv",
                survey.as_bytes(),
                Some("2026-11-09"),
                "t",
            )
            .unwrap();
        assert!(rep.accepted, "{}", findings(&rep));
        let st = s.state().unwrap();
        let p = st.piles.iter().find(|p| p.pile_id == p0.pile_id).unwrap();
        assert_eq!(p.z_lidar, Some(lidar), "LIDAR kept");
        assert!((p.z_survey.unwrap() - (lidar + 0.123)).abs() < 0.0006);
        assert_eq!(p.z_survey_date.as_deref(), Some("2026-11-09"));
        assert_eq!(p.z, p.z_survey, "the survey is in use");
        let csv = export::piles_csv(&st, &s.settings().unwrap());
        let head: Vec<&str> = csv.lines().next().unwrap().split(',').collect();
        let col = |name: &str| head.iter().position(|h| *h == name).unwrap();
        let row: Vec<&str> = csv
            .lines()
            .find(|l| l.starts_with(&format!("{},", p0.pile_id)))
            .unwrap()
            .split(',')
            .collect();
        assert_eq!(row[col("Survey - LIDAR (m)")], "0.12");
        assert_eq!(row[col("LIDAR Z")], format!("{lidar:.3}"));
    }

    #[test]
    fn flood_levels_follow_the_project_rule() {
        // Flat ground at 10.0, three piles under water: 0.10 m, 0.20 m and
        // 0.45 m deep (levels 10.10, 10.20, 10.45). Pile height 1.075 m gives
        // the module 0.30 m of clearance; 0.10 m freeboard is required.
        let mut s = Store::open_in_memory().unwrap();
        let mut master = String::from("Pile ID;Tracker;Easting;Northing;Z LIDAR\n");
        for i in 0..7 {
            master.push_str(&format!("P1.{i};T1;0;{};10,0\n", i * 8));
        }
        assert!(
            s.import_file(Kind::Master, "m.csv", master.as_bytes(), None, "t")
                .unwrap()
                .accepted
        );
        let mut set = s.settings().unwrap();
        assert_eq!(
            set.flood_mode,
            crate::FloodMode::Elevation,
            "levels by default"
        );
        assert!((set.flood_clearance() - 0.875).abs() < 1e-12);
        // Let the reveal go down to the rule's 1.075 m so the rule alone shows.
        set.delta_min = 1.075 - set.reveal_nominal;
        set.reveal_nominal = 1.075;
        set.delta_max = 0.5;
        s.set_settings(&set).unwrap();
        // Both columns: the level is used, the depth is not.
        let flood = "Pile ID;Flood depth (m);Flood level (m)\nP1.2;0,10;10,10\nP1.3;0,20;10,20\nP1.4;0,45;10,45\n";
        let rep = s
            .import_file(Kind::Flood, "flood.csv", flood.as_bytes(), None, "t")
            .unwrap();
        let text = findings(&rep);
        assert!(rep.accepted, "{text}");
        assert!(
            text.contains("Flood level read from column \"Flood level (m)\""),
            "{text}"
        );
        s.optimize(false, "t", 1).unwrap();
        let st = s.state().unwrap();
        let reveal = |id: &str| pile(&st, id).result.as_ref().unwrap().reveal;
        let min_top = |id: &str| pile(&st, id).min_top.unwrap();
        // Up to 0.20 m of water: no raise needed (minimum top at or below 1.075 m).
        assert!(min_top("P1.2") - 10.0 < 1.075 + 1e-9 && min_top("P1.3") - 10.0 < 1.075 + 1e-9);
        // 0.45 m of water: 0.45 - 0.20 = 0.25 m higher, pile 1.325 m, module
        // at 0.55 m, 0.10 m above the water.
        assert!(
            (min_top("P1.4") - 11.325).abs() < 1e-9,
            "{}",
            min_top("P1.4")
        );
        assert!(reveal("P1.4") >= 1.325 - 1e-6, "{}", reveal("P1.4"));
        assert!(pile(&st, "P1.4").result.as_ref().unwrap().min_top_active);
        // A depth-only file is refused while the settings ask for levels.
        let depths = "Pile ID;Flood depth\nP1.0;0,3\n";
        let rep = s
            .import_file(Kind::Flood, "d.csv", depths.as_bytes(), None, "t")
            .unwrap();
        assert!(
            !rep.accepted && findings(&rep).contains("gives flood depths"),
            "{}",
            findings(&rep)
        );
    }
}
