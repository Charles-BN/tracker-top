//! The master database: opening it, importing files, running the optimiser and
//! reading back the current state of every pile and tracker.

use crate::dates::find_date;
use crate::report::{ImportReport, Level};
use crate::settings::Settings;
use crate::table::{Col, Table, parse_num, read_table};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;
use top_core::{Pile, Tracker, default_solver, normalize_block, optimize_all};

pub type Result<T> = std::result::Result<T, String>;

/// A tracker's latest run: (run_id, created_at, status, input signature, max angle, max cut/fill).
type LatestRun = (i64, String, String, String, f64, f64);

const SCHEMA: &str = include_str!("schema.sql");
const SCHEMA_VERSION: &str = "1";

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
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Master => "master",
            Kind::Survey => "survey",
            Kind::Lidar => "lidar",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "master" => Some(Kind::Master),
            "survey" => Some(Kind::Survey),
            "lidar" => Some(Kind::Lidar),
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
    /// Ground elevation in use: the latest field survey, else the latest LIDAR.
    pub z: Option<f64>,
    pub z_source: Option<Kind>,
    pub z_import_id: Option<i64>,
    pub z_date: Option<String>,
    /// Surveyed position minus design position, horizontal distance (m).
    pub offset_m: Option<f64>,
    /// How many field surveys measured this pile (voided ones excluded).
    pub n_surveys: u32,
    pub result: Option<PileResultRow>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TrackerState {
    pub tracker: String,
    pub block: String,
    pub section: String,
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
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub db_path: String,
    pub piles: usize,
    pub trackers: usize,
    pub piles_surveyed: usize,
    pub piles_lidar_only: usize,
    pub piles_no_elevation: usize,
    pub tracker_status: BTreeMap<String, usize>,
    pub stale_trackers: usize,
    pub imports: usize,
    pub last_survey: Option<ImportRow>,
    pub last_run: Option<RunSummary>,
    pub blocks: Vec<BlockSummary>,
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
    conn: Connection,
    path: PathBuf,
}

impl Store {
    /// Open (or create) a database file.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(db)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(db)?;
        Self::init(conn, path.to_path_buf())
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(
            Connection::open_in_memory().map_err(db)?,
            PathBuf::from(":memory:"),
        )
    }

    fn init(conn: Connection, path: PathBuf) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON").map_err(db)?;
        conn.execute_batch(SCHEMA).map_err(db)?;
        let store = Self { conn, path };
        match store.meta("schema_version")? {
            None => store.set_meta("schema_version", SCHEMA_VERSION)?,
            Some(v) if v == SCHEMA_VERSION => {}
            Some(v) => {
                return Err(format!(
                    "database schema version {v} is not supported by this program"
                ));
            }
        }
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(db)
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
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
                serde_json::from_str(&json).map_err(|e| format!("stored settings are invalid: {e}"))
            }
            None => Ok(Settings::default()),
        }
    }

    pub fn set_settings(&self, s: &Settings) -> Result<()> {
        s.validate()?;
        self.set_meta(
            "settings",
            &serde_json::to_string(s).expect("settings serialise"),
        )
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
                "SELECT pile_id, tracker, block, section, easting, northing, pile_type, color
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
            rep.info("No header row found: columns were read as Point, Easting, Northing, Elevation, Code.");
        }
        let mut rep = match kind {
            Kind::Master => self.import_master(rep, &table, &sha, origin)?,
            Kind::Survey | Kind::Lidar => {
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
        let c_z = table.find(Col::Elevation);
        for (col, idx) in [
            (Col::PileId, c_id),
            (Col::Tracker, c_tracker),
            (Col::Block, c_block),
            (Col::Section, c_section),
            (Col::Easting, c_e),
            (Col::Northing, c_n),
            (Col::PileType, c_type),
            (Col::Color, c_color),
            (Col::Elevation, c_z),
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

        let mut rows: Vec<(MasterPile, Option<f64>)> = Vec::new();
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
                },
                z,
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
        for (p, z) in rows {
            match existing.get(&p.pile_id) {
                None => new_rows.push((p, z)),
                Some(old) => {
                    let identical = old.tracker == p.tracker
                        && old.block == p.block
                        && old.section == p.section
                        && old.pile_type == p.pile_type
                        && old.color == p.color
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

        let with_z = new_rows.iter().filter(|(_, z)| z.is_some()).count();
        let z_col = rep.columns.get("Elevation").cloned().unwrap_or_default();
        if with_z > 0 {
            rep.add(
                Level::Info,
                &format!("The file has a ground elevation column (\"{z_col}\"): {with_z} elevations were also recorded as LIDAR / design ground."),
                "",
            );
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
                    "INSERT INTO piles (pile_id, tracker, block, section, easting, northing, pile_type, color, import_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )
                .map_err(db)?;
            for (p, _) in &new_rows {
                ins.execute(params![
                    p.pile_id,
                    p.tracker,
                    p.block,
                    p.section,
                    p.easting,
                    p.northing,
                    p.pile_type,
                    p.color,
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
            for (p, z) in &new_rows {
                if let Some(z) = z {
                    ins.execute(params![lidar_id, p.pile_id, z]).map_err(db)?;
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
        let c_id = table.find(Col::PileId);
        let c_e = table.find(Col::Easting);
        let c_n = table.find(Col::Northing);
        let c_z = table.find(Col::Elevation);
        let c_code = table.find(Col::Code);
        let c_date = table.find(Col::Date);
        for (col, idx) in [
            (Col::PileId, c_id),
            (Col::Easting, c_e),
            (Col::Northing, c_n),
            (Col::Elevation, c_z),
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
                "No elevation column found (expected a header such as Elevation, Z, Height, Altitude or Z LIDAR). Headers in the file: {}.",
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
            rep.error("Points cannot be matched to piles: the file needs a pile ID column, or Easting and Northing columns.");
            return Ok(rep);
        }
        if !has_xy {
            rep.add(Level::Warning, "No Easting/Northing columns: points are matched by ID only and positions cannot be checked.", "");
        }

        let index = PileIndex::new(&masters, s.match_radius_m);
        let previous = if kind == Kind::Survey {
            self.latest_elevations()?
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
                    "Elevation missing or not a number: row skipped",
                    format!("{label} ({:?})", table.cell(row, c_z)),
                );
                continue;
            };
            let e = parse_num(table.cell(row, c_e));
            let n = parse_num(table.cell(row, c_n));

            let mut matched = (!id.is_empty())
                .then(|| index.by_id(id))
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
                "{} piles on {} trackers received a {} elevation dated {date}.",
                used.len(),
                trackers.len(),
                if kind == Kind::Survey {
                    "surveyed"
                } else {
                    "LIDAR"
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
        {
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
                "SELECT rt.tracker, rt.run_id, r.created_at, rt.status, rt.input_sig, rt.max_angle_deg, rt.max_cut_fill
                 FROM run_trackers rt JOIN runs r ON r.run_id = rt.run_id
                 ORDER BY rt.run_id",
            )
            .map_err(db)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ),
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
            .filter_map(|p| runs.get(&p.tracker).map(|r| (p.pile_id.as_str(), r.0)))
            .collect();
        let mut run_ids: Vec<i64> = runs.values().map(|r| r.0).collect();
        run_ids.sort_unstable();
        run_ids.dedup();
        let mut st = self
            .conn
            .prepare(
                "SELECT pile_id, run_id, z_ground, new_ground, cut_fill, top, reveal, angle_diff_deg, outlier
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
    fn input_sig(params_json: &str, piles: &[MasterPile], elevs: &[&Elev]) -> String {
        let mut h = Sha256::new();
        h.update(params_json.as_bytes());
        for (p, e) in piles.iter().zip(elevs) {
            h.update(p.pile_id.as_bytes());
            h.update([0]);
            h.update(p.easting.to_bits().to_le_bytes());
            h.update(p.northing.to_bits().to_le_bytes());
            h.update(e.z.to_bits().to_le_bytes());
            h.update(e.import_id.to_le_bytes());
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
        let started = Instant::now();
        let settings = self.settings()?;
        let params = settings.params();
        let params_json = settings.params_json();
        let piles = self.master_piles()?;
        let elevs = self.latest_elevations()?;
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
            let sig = Self::input_sig(&params_json, tp, &ev);
            if !force && latest.get(&tp[0].tracker).is_some_and(|l| l.3 == sig) {
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
            summary.duration_ms = started.elapsed().as_millis() as u64;
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
                    "INSERT INTO run_trackers (run_id, tracker, status, input_sig, n_survey, n_lidar, max_angle_deg, max_cut_fill)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .map_err(db)?;
            let mut ins_p = tx
                .prepare(
                    "INSERT INTO run_piles (run_id, pile_id, z_ground, z_import_id, new_ground, cut_fill, top, reveal, angle_diff_deg, outlier)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                        res.max_cut_fill
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
                            r.lidar_outlier as i64
                        ])
                        .map_err(db)?;
                }
            }
        }
        tx.commit().map_err(db)?;

        summary.run_id = Some(run_id);
        summary.created_at = created_at;
        summary.trackers_run = todo.len();
        summary.duration_ms = started.elapsed().as_millis() as u64;
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
        let runs = self.latest_runs()?;
        let mut results = self.latest_pile_results(&masters, &runs)?;
        let counts = self.survey_counts()?;

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
                )
            });
            let run = runs.get(&tp[0].tracker);
            let (status, stale) = match run {
                Some(r) => (r.2.clone(), current_sig.as_deref() != Some(r.3.as_str())),
                None if n_missing > 0 => ("AwaitingSurvey".to_string(), false),
                None => ("NotRun".to_string(), true),
            };
            trackers.push(TrackerState {
                tracker: tp[0].tracker.clone(),
                block: tp[0].block.clone(),
                section: tp[0].section.clone(),
                n_piles: tp.len(),
                n_survey,
                n_lidar,
                n_missing,
                status,
                stale,
                run_id: run.map(|r| r.0),
                run_at: run.map(|r| r.1.clone()),
                max_angle_deg: run.map(|r| r.4),
                max_cut_fill: run.map(|r| r.5),
                first_pile: range.start,
            });
            for (p, e) in tp.iter().zip(ev) {
                piles.push(PileState {
                    pile_id: p.pile_id.clone(),
                    tracker: p.tracker.clone(),
                    block: p.block.clone(),
                    section: p.section.clone(),
                    easting: p.easting,
                    northing: p.northing,
                    pile_type: p.pile_type.clone(),
                    color: p.color.clone(),
                    z: e.map(|e| e.z),
                    z_source: e.map(|e| e.kind),
                    z_import_id: e.map(|e| e.import_id),
                    z_date: e.map(|e| e.date.clone()),
                    offset_m: e.and_then(|e| e.offset_m),
                    n_surveys: counts.get(&p.pile_id).copied().unwrap_or(0),
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
        let state = self.state()?;
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
        let mut blocks: Vec<BlockSummary> = blocks.into_values().collect();
        blocks.sort_by_key(|a| natural_key(&a.block));
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
            stale_trackers: state.trackers.iter().filter(|t| t.stale).count(),
            tracker_status,
            imports: imports as usize,
            last_survey,
            last_run: self.last_run()?,
            blocks,
        })
    }
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
        let csv = export::piles_csv(&st);
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
}
