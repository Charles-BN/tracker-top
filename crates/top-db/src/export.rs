//! Exports of the current state: an Excel workbook (piles, trackers, imports,
//! settings) and a flat CSV of piles.

use crate::settings::Settings;
use crate::store::{ImportRow, Kind, PileState, State, TrackerState};
use rust_xlsxwriter::{Format, Workbook, Worksheet, XlsxError};
use serde::{Deserialize, Serialize};

/// One column of the Piles sheet. `key` is how export profiles and the page
/// name it; `group` sorts the columns in the export dialog.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Column {
    pub key: &'static str,
    pub header: &'static str,
    pub group: &'static str,
}

const fn col(key: &'static str, header: &'static str, group: &'static str) -> Column {
    Column { key, header, group }
}

/// Every column of the Piles sheet, in the order of `pile_cells`.
pub const COLUMNS: [Column; 33] = [
    col("pile_id", "Pile ID", "Pile"),
    col("block", "Block", "Pile"),
    col("section", "Section", "Pile"),
    col("tracker", "Tracker", "Pile"),
    col("tracker_type", "Tracker Type", "Pile"),
    col("pile_type", "Pile Type", "Pile"),
    col("color", "Color", "Pile"),
    col("easting", "Easting", "Position"),
    col("northing", "Northing", "Position"),
    col("set_easting", "Set-out Easting", "Position"),
    col("set_northing", "Set-out Northing", "Position"),
    col("shift", "Shift Along Tracker (m)", "Position"),
    col("ground_z", "Ground Z", "Ground"),
    col("z_source", "Z Source", "Ground"),
    col("z_date", "Z Date", "Ground"),
    col("survey_z", "Survey Z", "Ground"),
    col("lidar_z", "LIDAR Z", "Ground"),
    col("survey_minus_lidar", "Survey - LIDAR (m)", "Ground"),
    col("survey_offset", "Survey Offset (m)", "Ground"),
    col("surveys", "Surveys", "Ground"),
    col("flood_value", "Flood Value (m)", "Flood"),
    col("flood_level", "Flood Water Level", "Flood"),
    col("min_top", "Min ToP (flood)", "Flood"),
    col("raised", "Raised For Flood", "Flood"),
    col("new_ground", "NEW_ZG", "Result"),
    col("cut_fill", "Cut/Fill (m)", "Result"),
    col("top", "ToP", "Result"),
    col("reveal", "Reveal (m)", "Result"),
    col("angle", "Angle Diff (deg)", "Result"),
    col("status", "Tracker Status", "Result"),
    col("up_to_date", "Up To Date", "Result"),
    col("run", "Run", "Result"),
    col("comment", "Comment", "Result"),
];

/// Sheets of the workbook, in order.
pub const SHEETS: [&str; 4] = ["piles", "trackers", "imports", "settings"];

/// What to put in an export. Empty lists mean "everything".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSpec {
    /// Column keys of the Piles sheet (see `COLUMNS`), in the order wanted.
    pub columns: Vec<String>,
    /// Blocks to include (piles and trackers).
    pub blocks: Vec<String>,
    /// Sheets to include (see `SHEETS`).
    pub sheets: Vec<String>,
}

impl ExportSpec {
    /// Indices into `COLUMNS` of the columns wanted.
    pub fn column_indices(&self) -> Result<Vec<usize>, String> {
        if self.columns.is_empty() {
            return Ok((0..COLUMNS.len()).collect());
        }
        self.columns
            .iter()
            .map(|k| {
                COLUMNS
                    .iter()
                    .position(|c| c.key == k)
                    .ok_or_else(|| format!("unknown export column \"{k}\""))
            })
            .collect()
    }

    fn sheet(&self, name: &str) -> bool {
        self.sheets.is_empty() || self.sheets.iter().any(|s| s == name)
    }

    fn block(&self, block: &str) -> bool {
        self.blocks.is_empty() || self.blocks.iter().any(|b| b == block)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.column_indices()?;
        if let Some(s) = self.sheets.iter().find(|s| !SHEETS.contains(&s.as_str())) {
            return Err(format!("unknown sheet \"{s}\""));
        }
        Ok(())
    }
}

/// A saved choice of columns and sheets, shared by everyone using the database.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportProfile {
    pub name: String,
    pub columns: Vec<String>,
    pub sheets: Vec<String>,
}

/// Profiles of a new database: everything, and what installers need on site.
pub fn default_profiles() -> Vec<ExportProfile> {
    let keys = |k: &[&str]| k.iter().map(|s| s.to_string()).collect();
    vec![
        ExportProfile {
            name: "Full".into(),
            columns: Vec::new(),
            sheets: Vec::new(),
        },
        ExportProfile {
            name: "Installers".into(),
            columns: keys(&[
                "block",
                "tracker",
                "pile_id",
                "pile_type",
                "color",
                "set_easting",
                "set_northing",
                "ground_z",
                "top",
                "reveal",
                "status",
                "comment",
            ]),
            sheets: keys(&["piles"]),
        },
    ]
}

pub fn validate_profiles(profiles: &[ExportProfile]) -> Result<(), String> {
    let mut names = std::collections::HashSet::new();
    for p in profiles {
        let name = p.name.trim();
        if name.is_empty() {
            return Err("every export profile needs a name".into());
        }
        if !names.insert(name.to_lowercase()) {
            return Err(format!("two export profiles are called \"{name}\""));
        }
        ExportSpec {
            columns: p.columns.clone(),
            blocks: Vec::new(),
            sheets: p.sheets.clone(),
        }
        .validate()
        .map_err(|e| format!("export profile \"{name}\": {e}"))?;
    }
    Ok(())
}

enum Cell {
    Text(String),
    Num(f64, usize),
    Empty,
}

fn opt(v: Option<f64>, decimals: usize) -> Cell {
    v.map(|v| Cell::Num(v, decimals)).unwrap_or(Cell::Empty)
}

fn source(k: Option<Kind>) -> String {
    match k {
        Some(Kind::Survey) => "Survey".into(),
        Some(Kind::Lidar) => "LIDAR / design".into(),
        _ => String::new(),
    }
}

/// One row of the Piles sheet. `dc`: decimals of calculated values (2 when
/// they are rounded to the centimetre); measured values keep 3 (millimetre).
fn pile_cells(p: &PileState, t: &TrackerState, dc: usize) -> Vec<Cell> {
    let r = p.result.as_ref();
    let mut comment = Vec::new();
    if r.is_some_and(|r| r.outlier) {
        comment.push("Elevation outlier in tracker");
    }
    if t.stale && r.is_some() {
        comment.push("Result older than latest elevations/settings");
    }
    if p.z.is_none() {
        comment.push("No elevation yet");
    }
    if r.is_some_and(|r| r.min_top_active) {
        comment.push("Raised above flood level");
    }
    vec![
        Cell::Text(p.pile_id.clone()),
        Cell::Text(p.block.clone()),
        Cell::Text(p.section.clone()),
        Cell::Text(p.tracker.clone()),
        Cell::Text(p.tracker_type.clone()),
        Cell::Text(p.pile_type.clone()),
        Cell::Text(p.color.clone()),
        Cell::Num(p.easting, 4),
        Cell::Num(p.northing, 4),
        opt(r.and_then(|r| r.set_easting), dc),
        opt(r.and_then(|r| r.set_northing), dc),
        opt(r.and_then(|r| r.shift), dc),
        opt(p.z, 3),
        Cell::Text(source(p.z_source)),
        Cell::Text(p.z_date.clone().unwrap_or_default()),
        opt(p.z_survey, 3),
        opt(p.z_lidar, 3),
        opt(p.z_survey.zip(p.z_lidar).map(|(s, l)| s - l), dc),
        opt(p.offset_m, 3),
        Cell::Num(p.n_surveys as f64, 0),
        opt(p.flood, 3),
        opt(p.flood_level, dc),
        opt(p.min_top, dc),
        Cell::Text(match r {
            Some(r) if r.min_top_active => "Yes".into(),
            Some(_) if p.min_top.is_some() => "No".into(),
            _ => String::new(),
        }),
        opt(r.map(|r| r.new_ground), dc),
        opt(r.map(|r| r.cut_fill), dc),
        opt(r.map(|r| r.top), dc),
        opt(r.map(|r| r.reveal), dc),
        opt(r.map(|r| r.angle_diff_deg), 4),
        Cell::Text(t.status.clone()),
        Cell::Text(if t.run_id.is_none() {
            String::new()
        } else if t.stale {
            "No".into()
        } else {
            "Yes".into()
        }),
        opt(r.map(|r| r.run_id as f64), 0),
        Cell::Text(comment.join("; ")),
    ]
}

/// Rows of the Piles sheet: the blocks and columns of `spec`.
fn pile_rows<'a>(
    state: &'a State,
    spec: &'a ExportSpec,
    cols: &'a [usize],
    dc: usize,
) -> impl Iterator<Item = Vec<Cell>> + 'a {
    state
        .trackers
        .iter()
        .filter(|t| spec.block(&t.block))
        .flat_map(move |t| {
            state.piles[t.first_pile..t.first_pile + t.n_piles]
                .iter()
                .map(move |p| {
                    let mut all: Vec<Option<Cell>> =
                        pile_cells(p, t, dc).into_iter().map(Some).collect();
                    cols.iter()
                        .map(|&i| all[i].take().unwrap_or(Cell::Empty))
                        .collect()
                })
        })
}

fn headers(cols: &[usize]) -> Vec<&'static str> {
    cols.iter().map(|&i| COLUMNS[i].header).collect()
}

fn csv_quote(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// One row per pile, comma-separated, `.` decimal mark.
pub fn piles_csv(state: &State, settings: &Settings) -> String {
    piles_csv_with(state, &ExportSpec::default(), settings).expect("every column exists")
}

/// The CSV with the columns and blocks of `spec`.
pub fn piles_csv_with(
    state: &State,
    spec: &ExportSpec,
    settings: &Settings,
) -> Result<String, String> {
    let cols = spec.column_indices()?;
    let mut out = headers(&cols).join(",");
    out.push('\n');
    for row in pile_rows(state, spec, &cols, settings.decimals()) {
        let line: Vec<String> = row
            .iter()
            .map(|c| match c {
                Cell::Text(s) => csv_quote(s),
                Cell::Num(v, d) => format!("{v:.d$}", d = *d),
                Cell::Empty => String::new(),
            })
            .collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    Ok(out)
}

struct Formats {
    header: Format,
    num: [Format; 5],
}

impl Formats {
    fn new() -> Self {
        let num = |d: usize| {
            let f = if d == 0 {
                "0".to_string()
            } else {
                format!("0.{}", "0".repeat(d))
            };
            Format::new().set_num_format(f)
        };
        Self {
            header: Format::new().set_bold().set_background_color("#E8EEF4"),
            num: [num(0), num(1), num(2), num(3), num(4)],
        }
    }
}

fn write_sheet(
    ws: &mut Worksheet,
    f: &Formats,
    headers: &[&str],
    rows: impl Iterator<Item = Vec<Cell>>,
) -> Result<(), XlsxError> {
    for (c, h) in headers.iter().enumerate() {
        ws.write_string_with_format(0, c as u16, *h, &f.header)?;
        ws.set_column_width(c as u16, (h.len() as f64 + 2.0).max(10.0))?;
    }
    let mut last = 0u32;
    for (i, row) in rows.enumerate() {
        let r = i as u32 + 1;
        last = r;
        for (c, cell) in row.iter().enumerate() {
            match cell {
                Cell::Text(s) if !s.is_empty() => {
                    ws.write_string(r, c as u16, s)?;
                }
                Cell::Num(v, d) => {
                    ws.write_number_with_format(r, c as u16, *v, &f.num[(*d).min(4)])?;
                }
                _ => {}
            }
        }
    }
    ws.set_freeze_panes(1, 0)?;
    ws.autofilter(0, 0, last.max(1), headers.len() as u16 - 1)?;
    Ok(())
}

fn tracker_cells(t: &TrackerState, dc: usize) -> Vec<Cell> {
    vec![
        Cell::Text(t.tracker.clone()),
        Cell::Text(t.block.clone()),
        Cell::Text(t.section.clone()),
        Cell::Text(t.tracker_type.clone()),
        Cell::Num(t.n_piles as f64, 0),
        Cell::Num(t.n_survey as f64, 0),
        Cell::Num(t.n_lidar as f64, 0),
        Cell::Num(t.n_missing as f64, 0),
        Cell::Text(t.status.clone()),
        Cell::Text(if t.run_id.is_none() {
            String::new()
        } else if t.stale {
            "No".into()
        } else {
            "Yes".into()
        }),
        opt(t.max_angle_deg, 3),
        opt(t.max_cut_fill, dc),
        Cell::Text(t.anchor_pile.clone().unwrap_or_default()),
        opt(t.max_shift, dc),
        opt(t.span_error.map(|e| e * 1000.0), 3),
        Cell::Num(t.n_flood as f64, 0),
        Cell::Num(t.n_flood_raised as f64, 0),
        opt(t.tube_slope_pct, 2),
        Cell::Text(if t.steep { "Yes".into() } else { String::new() }),
        opt(t.run_id.map(|r| r as f64), 0),
        Cell::Text(t.run_at.clone().unwrap_or_default()),
    ]
}

fn import_cells(i: &ImportRow) -> Vec<Cell> {
    vec![
        Cell::Num(i.import_id as f64, 0),
        Cell::Text(i.kind.clone()),
        Cell::Text(i.file_name.clone()),
        Cell::Text(i.imported_at.clone()),
        Cell::Text(i.survey_date.clone().unwrap_or_default()),
        Cell::Text(i.origin.clone()),
        Cell::Num(i.rows_total as f64, 0),
        Cell::Num(i.rows_used as f64, 0),
        Cell::Num(i.rows_rejected as f64, 0),
        Cell::Text(i.voided_at.clone().unwrap_or_default()),
        Cell::Text(i.void_reason.clone().unwrap_or_default()),
    ]
}

/// Workbook with sheets Piles, Trackers, Imports and Settings.
pub fn workbook(
    state: &State,
    imports: &[ImportRow],
    settings: &Settings,
) -> Result<Vec<u8>, String> {
    workbook_with(state, imports, settings, &ExportSpec::default(), &[])
}

/// Workbook with the sheets, columns and blocks of `spec`. `about`, when not
/// empty, adds an About sheet of (label, value) lines after the Piles sheet.
pub fn workbook_with(
    state: &State,
    imports: &[ImportRow],
    settings: &Settings,
    spec: &ExportSpec,
    about: &[(String, String)],
) -> Result<Vec<u8>, String> {
    spec.validate()?;
    let cols = spec.column_indices()?;
    if cols.is_empty() && spec.sheet("piles") && spec.sheets.len() == 1 {
        return Err("choose at least one column".into());
    }
    let build = || -> Result<Vec<u8>, XlsxError> {
        let f = Formats::new();
        let mut wb = Workbook::new();

        if spec.sheet("piles") {
            let ws = wb.add_worksheet().set_name("Piles")?;
            write_sheet(
                ws,
                &f,
                &headers(&cols),
                pile_rows(state, spec, &cols, settings.decimals()),
            )?;
        }

        if !about.is_empty() {
            let ws = wb.add_worksheet().set_name("About")?;
            for (r, (k, v)) in about.iter().enumerate() {
                ws.write_string_with_format(r as u32, 0, k, &f.header)?;
                ws.write_string(r as u32, 1, v)?;
            }
            ws.set_column_width(0, 14)?;
            ws.set_column_width(1, 90)?;
        }

        if spec.sheet("trackers") {
            let ws = wb.add_worksheet().set_name("Trackers")?;
            write_sheet(
                ws,
                &f,
                &[
                    "Tracker",
                    "Block",
                    "Section",
                    "Tracker Type",
                    "Piles",
                    "Surveyed",
                    "LIDAR only",
                    "No elevation",
                    "Status",
                    "Up To Date",
                    "Max Angle (deg)",
                    "Max Cut/Fill (m)",
                    "Fixed Pile (set-out)",
                    "Max Shift (m)",
                    "Span Check (mm)",
                    "Flood Values",
                    "Raised For Flood",
                    "Tube Slope (%)",
                    "Slope Warning",
                    "Run",
                    "Run Date",
                ],
                state
                    .trackers
                    .iter()
                    .filter(|t| spec.block(&t.block))
                    .map(|t| tracker_cells(t, settings.decimals())),
            )?;
        }

        if spec.sheet("imports") {
            let ws = wb.add_worksheet().set_name("Imports")?;
            write_sheet(
                ws,
                &f,
                &[
                    "Import",
                    "Kind",
                    "File",
                    "Imported At",
                    "Survey Date",
                    "Origin",
                    "Rows",
                    "Used",
                    "Rejected",
                    "Voided At",
                    "Void Reason",
                ],
                imports.iter().map(import_cells),
            )?;
        }

        if spec.sheet("settings") {
            let ws = wb.add_worksheet().set_name("Settings")?;
            let json = serde_json::to_value(settings).expect("settings serialise");
            let rows = json
                .as_object()
                .map(|o| {
                    o.iter()
                        .map(|(k, v)| {
                            vec![
                                Cell::Text(k.clone()),
                                v.as_f64()
                                    .map(|v| Cell::Num(v, 4))
                                    .unwrap_or(Cell::Text(v.to_string())),
                            ]
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            write_sheet(ws, &f, &["Setting", "Value"], rows.into_iter())?;
            ws.set_column_width(0, 24)?;
        }

        if wb.worksheets().is_empty() {
            wb.add_worksheet().set_name("Piles")?;
        }
        wb.save_to_buffer()
    };
    build().map_err(|e| format!("cannot write the Excel file: {e}"))
}
