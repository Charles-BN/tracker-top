//! Minimal CSV reading and writing, using the same column names as the
//! original Python workflow so results can drop into existing spreadsheets.

use crate::error::{Error, Result};
use crate::model::{normalize_block, Pile, Tracker};
use crate::optimize::TrackerResult;
use crate::params::Params;
use std::io::{BufRead, Write};

/// Split one CSV line into fields. Supports double-quoted fields with `""` escapes.
pub fn split_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, in_quotes) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            ('"', _) => in_quotes = !in_quotes,
            (',', false) => fields.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    fields.push(cur);
    fields
}

fn quote(field: &str) -> String {
    if field.contains([',', '"', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Read piles from CSV. Required columns: `Pile Name`, `Tracker`, `Easting`,
/// `Northing`, `Z LIDAR`. Optional: `Block`, `Section`, `Pile Type`.
pub fn read_piles<R: BufRead>(reader: R) -> Result<Vec<Pile>> {
    let mut lines = reader.lines();
    let header_line = lines
        .next()
        .ok_or_else(|| Error::Invalid("input is empty".into()))??;
    let header: Vec<String> = split_line(header_line.trim_start_matches('\u{feff}'))
        .into_iter()
        .map(|h| h.trim().to_string())
        .collect();
    let find = |name: &str| header.iter().position(|h| h == name);
    let need = |name: &str| find(name).ok_or_else(|| Error::MissingColumn(name.to_string()));

    let c_name = need("Pile Name")?;
    let c_tracker = need("Tracker")?;
    let c_e = need("Easting")?;
    let c_n = need("Northing")?;
    let c_z = need("Z LIDAR")?;
    let c_block = find("Block");
    let c_section = find("Section");
    let c_type = find("Pile Type");

    let mut piles = Vec::new();
    for (idx, line) in lines.enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let line_no = idx + 2;
        let f = split_line(&line);
        let get = |c: usize| f.get(c).map(|s| s.trim()).unwrap_or("");
        let num = |c: usize, col: &str| -> Result<f64> {
            get(c).parse::<f64>().map_err(|_| Error::BadValue {
                line: line_no,
                column: col.to_string(),
                value: get(c).to_string(),
            })
        };
        piles.push(Pile {
            name: get(c_name).to_string(),
            tracker: get(c_tracker).to_string(),
            block: c_block.map(|c| normalize_block(get(c))).unwrap_or_default(),
            section: c_section.map(|c| get(c).to_string()).unwrap_or_default(),
            pile_type: c_type.map(|c| get(c).to_string()).unwrap_or_default(),
            easting: num(c_e, "Easting")?,
            northing: num(c_n, "Northing")?,
            z_lidar: num(c_z, "Z LIDAR")?,
            min_top: None,
        });
    }
    Ok(piles)
}

/// Write one row per pile. Column names follow the Python script's output.
pub fn write_results<W: Write>(
    mut w: W,
    trackers: &[Tracker],
    results: &[TrackerResult],
    params: &Params,
) -> Result<()> {
    writeln!(
        w,
        "Pile Name,Block,Section,Tracker,Pile Type,Easting,Northing,Z LIDAR,NEW_ZG,CF,\
Optimized ToP,Final Reveal,Max_Angle,Angle_Diff,Comment,Status"
    )?;
    for (tr, res) in trackers.iter().zip(results) {
        for (p, r) in tr.piles.iter().zip(&res.piles) {
            writeln!(
                w,
                "{},{},{},{},{},{:.4},{:.4},{:.4},{:.5},{:.5},{:.5},{:.5},{},{:.4},{},{}",
                quote(&p.name),
                quote(&p.block),
                quote(&p.section),
                quote(&tr.id),
                quote(&p.pile_type),
                p.easting,
                p.northing,
                p.z_lidar,
                r.new_ground,
                r.cut_fill,
                r.top,
                r.final_reveal,
                params.max_angle_deg,
                r.angle_diff_deg,
                if r.lidar_outlier { "LIDAR flaw" } else { "" },
                res.status.as_str(),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn splits_quoted_fields() {
        assert_eq!(split_line("a,\"b,c\",\"d\"\"e\",,f"), vec!["a", "b,c", "d\"e", "", "f"]);
    }

    #[test]
    fn reads_piles_with_optional_columns_missing() {
        let csv = "Pile Name,Tracker,Easting,Northing,Z LIDAR\nP1,T1,10.5,20.25,99.5\n\nP2,T1,10.5,28.25,99.75\n";
        let piles = read_piles(Cursor::new(csv)).unwrap();
        assert_eq!(piles.len(), 2);
        assert_eq!(piles[1].z_lidar, 99.75);
        assert_eq!(piles[0].block, "");
    }

    #[test]
    fn reports_missing_columns_and_bad_values() {
        let err = read_piles(Cursor::new("Pile Name,Tracker,Easting,Northing\n")).unwrap_err();
        assert!(matches!(err, Error::MissingColumn(c) if c == "Z LIDAR"));
        let csv = "Pile Name,Tracker,Easting,Northing,Z LIDAR\nP1,T1,1,2,abc\n";
        let err = read_piles(Cursor::new(csv)).unwrap_err();
        assert!(matches!(err, Error::BadValue { line: 2, .. }));
    }
}
