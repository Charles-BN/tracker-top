//! Exports of the current state: an Excel workbook (piles, trackers, imports,
//! settings) and a flat CSV of piles.

use crate::settings::Settings;
use crate::store::{ImportRow, Kind, PileState, State, TrackerState};
use rust_xlsxwriter::{Format, Workbook, Worksheet, XlsxError};

const PILE_HEADERS: [&str; 27] = [
    "Pile ID",
    "Block",
    "Section",
    "Tracker",
    "Tracker Type",
    "Pile Type",
    "Color",
    "Easting",
    "Northing",
    "Ground Z",
    "Z Source",
    "Z Date",
    "Survey Offset (m)",
    "Surveys",
    "Flood Value (m)",
    "Flood Water Level",
    "Min ToP (flood)",
    "Raised For Flood",
    "NEW_ZG",
    "Cut/Fill (m)",
    "ToP",
    "Reveal (m)",
    "Angle Diff (deg)",
    "Tracker Status",
    "Up To Date",
    "Run",
    "Comment",
];

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

fn pile_cells(p: &PileState, t: &TrackerState) -> Vec<Cell> {
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
        opt(p.z, 4),
        Cell::Text(source(p.z_source)),
        Cell::Text(p.z_date.clone().unwrap_or_default()),
        opt(p.offset_m, 3),
        Cell::Num(p.n_surveys as f64, 0),
        opt(p.flood, 3),
        opt(p.flood_level, 4),
        opt(p.min_top, 4),
        Cell::Text(match r {
            Some(r) if r.min_top_active => "Yes".into(),
            Some(_) if p.min_top.is_some() => "No".into(),
            _ => String::new(),
        }),
        opt(r.map(|r| r.new_ground), 4),
        opt(r.map(|r| r.cut_fill), 4),
        opt(r.map(|r| r.top), 4),
        opt(r.map(|r| r.reveal), 4),
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

fn pile_rows(state: &State) -> impl Iterator<Item = Vec<Cell>> + '_ {
    state.trackers.iter().flat_map(move |t| {
        state.piles[t.first_pile..t.first_pile + t.n_piles]
            .iter()
            .map(move |p| pile_cells(p, t))
    })
}

fn csv_quote(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// One row per pile, comma-separated, `.` decimal mark.
pub fn piles_csv(state: &State) -> String {
    let mut out = PILE_HEADERS.join(",");
    out.push('\n');
    for row in pile_rows(state) {
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
    out
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

fn tracker_cells(t: &TrackerState) -> Vec<Cell> {
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
        opt(t.max_cut_fill, 3),
        Cell::Num(t.n_flood as f64, 0),
        Cell::Num(t.n_flood_raised as f64, 0),
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
    let build = || -> Result<Vec<u8>, XlsxError> {
        let f = Formats::new();
        let mut wb = Workbook::new();

        let ws = wb.add_worksheet().set_name("Piles")?;
        write_sheet(ws, &f, &PILE_HEADERS, pile_rows(state))?;

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
                "Flood Values",
                "Raised For Flood",
                "Run",
                "Run Date",
            ],
            state.trackers.iter().map(tracker_cells),
        )?;

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

        wb.save_to_buffer()
    };
    build().map_err(|e| format!("cannot write the Excel file: {e}"))
}
