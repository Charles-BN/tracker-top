//! Reading tabular files into rows of text, and finding columns by name.
//!
//! Survey and pile-list files come from many tools, so the reader is lenient:
//!
//! - CSV / TXT with `,` `;` tab or whitespace separators (detected), optional quotes,
//!   UTF-8 or Windows-1252 text, decimal commas (`51,051`).
//! - Excel / OpenDocument (`.xlsx`, `.xlsm`, `.xls`, `.ods`): first worksheet.
//! - Files with no header row, as exported by survey controllers
//!   (`Point, Easting, Northing, Elevation, Code`, "PENZD").

use std::io::Cursor;

/// A file read as text cells. `headers` always has one entry per column.
#[derive(Clone, Debug, Default)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// False when the file had no header row and positional PENZD names were assumed.
    pub had_header: bool,
}

/// Logical columns the importers look for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Col {
    PileId,
    Tracker,
    Block,
    Section,
    Easting,
    Northing,
    Elevation,
    PileType,
    Color,
    Date,
    Code,
    TrackerType,
    Flood,
}

impl Col {
    pub fn label(self) -> &'static str {
        match self {
            Col::PileId => "Pile ID",
            Col::Tracker => "Tracker",
            Col::Block => "Block",
            Col::Section => "Section",
            Col::Easting => "Easting",
            Col::Northing => "Northing",
            Col::Elevation => "Elevation",
            Col::PileType => "Pile Type",
            Col::Color => "Color",
            Col::Date => "Date",
            Col::Code => "Code",
            Col::TrackerType => "Tracker Type",
            Col::Flood => "Flood",
        }
    }

    /// Accepted header spellings, compared after [`norm_header`]. Earlier entries win.
    fn aliases(self) -> &'static [&'static str] {
        match self {
            Col::PileId => &[
                "pileid",
                "pilename",
                "pile",
                "pileno",
                "pilenumber",
                "pointid",
                "pointname",
                "point",
                "pointnumber",
                "pt",
                "ptid",
                "ptname",
                "name",
                "id",
                "nom",
                "nompieu",
                "pieu",
                "identifiant",
                "numero",
                "number",
            ],
            Col::Tracker => &[
                "tracker",
                "trackerid",
                "trackername",
                "table",
                "tableid",
                "rangee",
            ],
            Col::Block => &["block", "bloc", "blockid"],
            Col::Section => &["section", "sectionid"],
            Col::Easting => &[
                "easting",
                "east",
                "designeasting",
                "e",
                "x",
                "est",
                "coordx",
                "xcoord",
                "xm",
                "em",
            ],
            Col::Northing => &[
                "northing",
                "north",
                "designnorthing",
                "n",
                "y",
                "nord",
                "coordy",
                "ycoord",
                "ym",
                "nm",
            ],
            Col::Elevation => &[
                "elevation",
                "groundelevation",
                "zground",
                "groundz",
                "zsurvey",
                // Design ground (LIDAR with planned cut/fill) wins over raw LIDAR.
                "zlidarcf",
                "zdesign",
                "designz",
                "designground",
                "zlidar",
                "lidarz",
                "lidar",
                "elev",
                "height",
                "altitude",
                "alt",
                "z",
                "zm",
                "ztn",
                "tn",
                "zngf",
                "ngf",
            ],
            Col::PileType => &["piletype", "type", "typepieu", "pieutype", "pilekind"],
            Col::TrackerType => &[
                "trackertype",
                "typetracker",
                "trackermodel",
                "trackerkind",
                "typedetracker",
            ],
            Col::Flood => &[
                "flood",
                "flooddepth",
                "floodlevel",
                "floodelevation",
                "floodheight",
                "floodz",
                "zflood",
                "floodvalue",
                "inondation",
                "hauteurinondation",
                "niveauinondation",
                "coteinondation",
                "phe",
                "zphe",
                "waterlevel",
                "waterdepth",
                "wse",
            ],
            Col::Color => &["color", "colour", "colorcode", "colourcode", "couleur"],
            Col::Date => &[
                "surveydate",
                "date",
                "datetime",
                "measured",
                "measuredat",
                "time",
            ],
            Col::Code => &[
                "code",
                "featurecode",
                "feature",
                "description",
                "desc",
                "remark",
            ],
        }
    }
}

/// Lower-case, accents folded, only letters and digits: `"Z LIDAR (m)"` -> `"zlidarm"`.
pub fn norm_header(h: &str) -> String {
    h.chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'À' | 'Â' | 'Ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
            'î' | 'ï' | 'Î' | 'Ï' => 'i',
            'ô' | 'ö' | 'Ô' | 'Ö' => 'o',
            'ù' | 'û' | 'ü' | 'Ù' | 'Û' | 'Ü' => 'u',
            'ç' | 'Ç' => 'c',
            other => other,
        })
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// A normalised header matches an alias exactly, or with a trailing metre
/// unit: `"Flood depth (m)"` -> `"flooddepthm"` matches `"flooddepth"`.
fn header_is(h: &str, alias: &str) -> bool {
    h == alias || (h.len() == alias.len() + 1 && h.starts_with(alias) && h.ends_with('m'))
}

impl Table {
    /// Column index for a logical column, if any header matches one of its aliases.
    pub fn find(&self, col: Col) -> Option<usize> {
        let normed: Vec<String> = self.headers.iter().map(|h| norm_header(h)).collect();
        col.aliases()
            .iter()
            .find_map(|alias| normed.iter().position(|h| header_is(h, alias)))
    }

    /// Every column whose header is one of `col`'s spellings, in file order.
    pub fn find_all(&self, col: Col) -> Vec<usize> {
        self.headers
            .iter()
            .enumerate()
            .filter(|(_, h)| {
                let h = norm_header(h);
                col.aliases().iter().any(|a| header_is(&h, a))
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn cell<'a>(&'a self, row: &'a [String], col: Option<usize>) -> &'a str {
        col.and_then(|c| row.get(c)).map(|s| s.trim()).unwrap_or("")
    }
}

/// Parse a number written with `.` or `,` as decimal mark, ignoring spaces used
/// as thousands separators. `"305 512,59"` -> 305512.59, `"1,234.5"` -> 1234.5.
pub fn parse_num(raw: &str) -> Option<f64> {
    let s: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{a0}' && *c != '\u{202f}')
        .collect();
    if s.is_empty() {
        return None;
    }
    let s = match (s.contains('.'), s.contains(',')) {
        (true, true) => s.replace(',', ""),
        (false, true) => s.replace(',', "."),
        _ => s,
    };
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Read a file, choosing the format from its extension.
pub fn read_table(file_name: &str, bytes: &[u8]) -> Result<Table, String> {
    let ext = file_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let raw_rows = match ext.as_str() {
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => read_spreadsheet(bytes)?,
        _ => read_delimited(&decode_text(bytes)),
    };
    Ok(finish(raw_rows))
}

fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        // Windows-1252 / Latin-1 fallback: the printable range maps 1:1 to Unicode
        // for every character a survey export is likely to contain.
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

fn read_spreadsheet(bytes: &[u8]) -> Result<Vec<Vec<String>>, String> {
    use calamine::{Data, Reader, open_workbook_auto_from_rs};
    let mut wb = open_workbook_auto_from_rs(Cursor::new(bytes.to_vec()))
        .map_err(|e| format!("cannot open spreadsheet: {e}"))?;
    let range = wb
        .worksheet_range_at(0)
        .ok_or("the workbook has no worksheet")?
        .map_err(|e| format!("cannot read the first worksheet: {e}"))?;
    Ok(range
        .rows()
        .map(|r| {
            r.iter()
                .map(|c| match c {
                    Data::Empty => String::new(),
                    Data::Float(f) => format!("{f}"),
                    Data::Int(i) => format!("{i}"),
                    Data::DateTime(d) => excel_serial_to_text(d.as_f64()),
                    other => other.to_string(),
                })
                .collect()
        })
        .collect())
}

/// Excel date serial (days since 1899-12-30) as `YYYY-MM-DD HH:MM:SS`.
fn excel_serial_to_text(serial: f64) -> String {
    let base = chrono::NaiveDate::from_ymd_opt(1899, 12, 30)
        .expect("valid date")
        .and_hms_opt(0, 0, 0)
        .expect("valid time");
    let secs = (serial * 86_400.0).round();
    if !secs.is_finite() || secs.abs() > 1e11 {
        return format!("{serial}");
    }
    (base + chrono::Duration::seconds(secs as i64))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// Count separators outside quotes on a line.
fn count_outside_quotes(line: &str, sep: char) -> usize {
    let mut in_q = false;
    let mut n = 0;
    for c in line.chars() {
        if c == '"' {
            in_q = !in_q;
        } else if c == sep && !in_q {
            n += 1;
        }
    }
    n
}

fn detect_separator(lines: &[&str]) -> Option<char> {
    let sample: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .take(20)
        .collect();
    // A separator counts only if it appears on every sampled line. `;` and tab
    // win over `,` because `,` may be a decimal mark in `;`-separated files.
    [';', '\t', ',', '|']
        .into_iter()
        .map(|sep| {
            let min = sample
                .iter()
                .map(|l| count_outside_quotes(l, sep))
                .min()
                .unwrap_or(0);
            (sep, min)
        })
        .find(|&(_, min)| min > 0)
        .map(|(sep, _)| sep)
}

fn split_delimited(line: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if in_q && chars.peek() == Some(&'"') {
                cur.push('"');
                chars.next();
            } else {
                in_q = !in_q;
            }
        } else if c == sep && !in_q {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn read_delimited(text: &str) -> Vec<Vec<String>> {
    let lines: Vec<&str> = text.lines().collect();
    let sep = detect_separator(&lines);
    lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| match sep {
            Some(s) => split_delimited(l.trim_end_matches('\r'), s),
            None => l.split_whitespace().map(str::to_string).collect(),
        })
        .collect()
}

/// Decide whether the first row is a header; pad rows to a common width.
fn finish(mut rows: Vec<Vec<String>>) -> Table {
    rows.retain(|r| r.iter().any(|c| !c.trim().is_empty()));
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for r in &mut rows {
        r.resize(width, String::new());
        for c in r.iter_mut() {
            *c = c.trim().to_string();
        }
    }
    let Some(first) = rows.first() else {
        return Table::default();
    };
    // A header row has at most one numeric cell; a PENZD data row has at least three.
    let numeric = first.iter().filter(|c| parse_num(c).is_some()).count();
    if numeric <= 1 {
        let headers = rows.remove(0);
        Table {
            headers,
            rows,
            had_header: true,
        }
    } else {
        // Named by position only; the importer recognises them from their values.
        let headers = (0..width).map(|i| format!("Column {}", i + 1)).collect();
        Table {
            headers,
            rows,
            had_header: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_with_either_decimal_mark() {
        assert_eq!(parse_num("51.25"), Some(51.25));
        assert_eq!(parse_num(" 51,25 "), Some(51.25));
        assert_eq!(parse_num("305 512,5968"), Some(305512.5968));
        assert_eq!(parse_num("1,234.5"), Some(1234.5));
        assert_eq!(parse_num(""), None);
        assert_eq!(parse_num("abc"), None);
        assert_eq!(parse_num("NaN"), None);
    }

    #[test]
    fn semicolon_csv_with_decimal_commas() {
        let t = read_table(
            "s.csv",
            "Nom;X;Y;Z TN\nP1;305512,5;6428719,7;51,05\n".as_bytes(),
        )
        .unwrap();
        assert!(t.had_header);
        assert_eq!(t.find(Col::PileId), Some(0));
        assert_eq!(t.find(Col::Easting), Some(1));
        assert_eq!(t.find(Col::Northing), Some(2));
        assert_eq!(t.find(Col::Elevation), Some(3));
        assert_eq!(parse_num(&t.rows[0][3]), Some(51.05));
    }

    #[test]
    fn headerless_penzd_export() {
        let t = read_table(
            "pts.txt",
            b"P01.02.01.05,305512.597,6428719.713,51.051,GND\r\nP2,1,2,3,GND\r\n",
        )
        .unwrap();
        assert!(!t.had_header);
        assert_eq!(t.rows.len(), 2);
        // No names to go by: the importer recognises the columns from their values.
        assert_eq!(t.headers[0], "Column 1");
        assert_eq!(t.find(Col::PileId), None);
    }

    #[test]
    fn whitespace_separated_and_latin1() {
        let mut bytes = b"Point Est Nord Altitude\n".to_vec();
        bytes.extend_from_slice(b"P\xe91 10.0 20.0 30.0\n");
        let t = read_table("pts.txt", &bytes).unwrap();
        assert_eq!(t.headers, vec!["Point", "Est", "Nord", "Altitude"]);
        assert_eq!(t.rows[0][0], "Pé1");
        assert_eq!(t.find(Col::Easting), Some(1));
        assert_eq!(t.find(Col::Elevation), Some(3));
    }

    #[test]
    fn quoted_fields_and_bom() {
        let t = read_table(
            "a.csv",
            "\u{feff}\"Pile Name\",Easting,Northing,\"Z LIDAR\"\n\"P,1\",1,2,3\n".as_bytes(),
        )
        .unwrap();
        assert_eq!(t.rows[0][0], "P,1");
        assert_eq!(t.find(Col::PileId), Some(0));
        assert_eq!(t.find(Col::Elevation), Some(3));
    }

    #[test]
    fn design_ground_flood_and_units() {
        let t = Table {
            headers: [
                "Pile Name",
                "Tracker type",
                "Z LIDAR_CF",
                "Z LIDAR",
                "Z CF",
                "Flood depth (m)",
                "Northing (m)",
            ]
            .map(String::from)
            .to_vec(),
            rows: vec![],
            had_header: true,
        };
        assert_eq!(t.find(Col::Elevation), Some(2));
        assert_eq!(t.find_all(Col::Elevation), vec![2, 3]);
        assert_eq!(t.find(Col::TrackerType), Some(1));
        assert_eq!(t.find(Col::PileType), None);
        assert_eq!(t.find(Col::Tracker), None);
        assert_eq!(t.find(Col::Flood), Some(5));
        assert_eq!(t.find(Col::Northing), Some(6));
    }

    #[test]
    fn header_aliases_prefer_specific_names() {
        let t = Table {
            headers: vec![
                "Name".into(),
                "Pile ID".into(),
                "Z".into(),
                "Z LIDAR".into(),
            ],
            rows: vec![],
            had_header: true,
        };
        assert_eq!(t.find(Col::PileId), Some(1));
        assert_eq!(t.find(Col::Elevation), Some(3));
    }
}
