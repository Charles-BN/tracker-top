//! Blocks ready for the installers, and the record of the files sent to them.
//!
//! A block is ready when every one of its piles has a field-survey elevation
//! and every tracker in it has a current optimisation result (and, by default,
//! every tracker is Compliant or Graded). Its file is then written to the
//! installers' folder, once. If a later survey changes the block's set-out
//! positions or heights, a new revision of the file is written.

use crate::export::{self, ExportProfile, ExportSpec};
use crate::store::{Result, State, Store};
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// A file sent to the installers.
#[derive(Clone, Debug, Serialize)]
pub struct BlockExport {
    pub revision: i64,
    pub content_sha: String,
    pub file_name: String,
    pub exported_at: String,
    pub exported_by: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlockStatus {
    pub block: String,
    pub piles: usize,
    pub surveyed: usize,
    pub trackers: usize,
    /// Trackers with a result that matches the current elevations and settings.
    pub current: usize,
    /// Of those, Compliant or Graded.
    pub ok: usize,
    pub ready: bool,
    /// Why the block is not ready, for the page.
    pub waiting: String,
    /// Fingerprint of what the installers' file contains (positions, tops, status).
    pub content_sha: Option<String>,
    pub last_export: Option<BlockExport>,
    /// Ready, and different from the last file sent.
    pub needs_export: bool,
    /// Revision number the next file gets.
    pub next_revision: i64,
    pub reveal_mean: Option<f64>,
}

/// Fingerprint of a block's results: pile, set-out position and top, to the
/// millimetre, and tracker status. Re-optimising with the same results keeps it.
pub fn content_sha(state: &State, block: &str) -> String {
    let mut h = Sha256::new();
    for t in state.trackers.iter().filter(|t| t.block == block) {
        h.update(format!("{}|{}\n", t.tracker, t.status));
        for p in &state.piles[t.first_pile..t.first_pile + t.n_piles] {
            let r = p.result.as_ref();
            let mm = |v: Option<f64>| v.map(|v| format!("{v:.3}")).unwrap_or_default();
            h.update(format!(
                "{}|{}|{}|{}\n",
                p.pile_id,
                mm(r.and_then(|r| r.set_easting).or(Some(p.easting))),
                mm(r.and_then(|r| r.set_northing).or(Some(p.northing))),
                mm(r.map(|r| r.top)),
            ));
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// A block name usable in a file name on Windows.
pub fn safe_name(block: &str) -> String {
    let b: String = block
        .chars()
        .map(|c| {
            if "\\/:*?\"<>|".contains(c) || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let b = b.trim();
    if b.is_empty() {
        "(no block)".into()
    } else {
        b.into()
    }
}

/// File name of a block's export.
pub fn file_name(block: &str, revision: i64) -> String {
    format!(
        "Block {} - pile heights rev {revision}.xlsx",
        safe_name(block)
    )
}

impl Store {
    fn block_exports(&self) -> Result<Vec<(String, BlockExport)>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT block, revision, content_sha, file_name, exported_at, exported_by
                 FROM block_exports ORDER BY revision, export_id",
            )
            .map_err(|e| e.to_string())?;
        st.query_map([], |r| {
            Ok((
                r.get(0)?,
                BlockExport {
                    revision: r.get(1)?,
                    content_sha: r.get(2)?,
                    file_name: r.get(3)?,
                    exported_at: r.get(4)?,
                    exported_by: r.get(5)?,
                },
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())
    }

    /// Every block, with its readiness for the installers.
    pub fn block_status(&self) -> Result<Vec<BlockStatus>> {
        let state = self.state()?;
        self.block_status_of(&state)
    }

    pub(crate) fn block_status_of(&self, state: &State) -> Result<Vec<BlockStatus>> {
        let require_ok = self.settings()?.auto_export_require_ok;
        let mut last: std::collections::HashMap<String, BlockExport> = Default::default();
        for (block, e) in self.block_exports()? {
            last.insert(block, e); // ordered by revision: the latest stays
        }
        let mut blocks: Vec<String> = state.trackers.iter().map(|t| t.block.clone()).collect();
        blocks.sort_by_key(|b| crate::store::natural_key(b));
        blocks.dedup();
        let mut out = Vec::new();
        for block in blocks {
            let trackers: Vec<_> = state.trackers.iter().filter(|t| t.block == block).collect();
            let piles = trackers.iter().map(|t| t.n_piles).sum::<usize>();
            let surveyed = trackers.iter().map(|t| t.n_survey).sum::<usize>();
            let current = trackers
                .iter()
                .filter(|t| t.run_id.is_some() && !t.stale)
                .count();
            let ok = trackers
                .iter()
                .filter(|t| t.run_id.is_some() && !t.stale)
                .filter(|t| t.status == "Compliant" || t.status == "Graded")
                .count();
            let waiting = if surveyed < piles {
                format!("{} piles not surveyed yet", piles - surveyed)
            } else if current < trackers.len() {
                format!("{} trackers to optimise", trackers.len() - current)
            } else if require_ok && ok < trackers.len() {
                format!(
                    "{} trackers need attention (not compliant)",
                    trackers.len() - ok
                )
            } else {
                String::new()
            };
            let ready = waiting.is_empty();
            let sha = ready.then(|| content_sha(state, &block));
            let last_export = last.remove(&block);
            let needs_export =
                ready && last_export.as_ref().map(|e| &e.content_sha) != sha.as_ref();
            let next_revision = match &last_export {
                Some(e) if needs_export => e.revision + 1,
                Some(e) => e.revision,
                None => 1,
            };
            let reveals: Vec<f64> = trackers
                .iter()
                .flat_map(|t| &state.piles[t.first_pile..t.first_pile + t.n_piles])
                .filter_map(|p| p.result.as_ref().map(|r| r.reveal))
                .collect();
            out.push(BlockStatus {
                block,
                piles,
                surveyed,
                trackers: trackers.len(),
                current,
                ok,
                ready,
                waiting,
                content_sha: sha,
                last_export,
                needs_export,
                next_revision,
                reveal_mean: mean(&reveals),
            });
        }
        Ok(out)
    }

    /// The export profile named `name`, or the built-in one of that name.
    pub fn export_profile(&self, name: &str) -> Result<ExportProfile> {
        self.export_profiles()?
            .into_iter()
            .chain(export::default_profiles())
            .find(|p| p.name.eq_ignore_ascii_case(name.trim()))
            .ok_or_else(|| format!("there is no export profile called \"{name}\""))
    }

    pub fn export_profiles(&self) -> Result<Vec<ExportProfile>> {
        match self.meta("export_profiles")? {
            Some(json) => serde_json::from_str(&json)
                .map_err(|e| format!("stored export profiles are invalid: {e}")),
            None => Ok(export::default_profiles()),
        }
    }

    pub fn set_export_profiles(&self, profiles: &[ExportProfile]) -> Result<()> {
        export::validate_profiles(profiles)?;
        if self.meta("export_profiles")?.is_some() && self.export_profiles()? == profiles {
            return Ok(());
        }
        self.set_meta(
            "export_profiles",
            &serde_json::to_string(profiles).expect("profiles serialise"),
        )?;
        self.set_meta("export_profiles_at", &crate::sync::utc_now())
    }

    /// The installers' workbook of a block, and its file name.
    pub fn block_workbook(&self, block: &str) -> Result<(String, Vec<u8>)> {
        let state = self.state()?;
        let settings = self.settings()?;
        let status = self
            .block_status_of(&state)?
            .into_iter()
            .find(|b| b.block == block)
            .ok_or_else(|| format!("there is no block \"{block}\""))?;
        let profile = self.export_profile(&settings.auto_export_profile)?;
        let spec = ExportSpec {
            columns: profile.columns,
            blocks: vec![block.to_string()],
            sheets: profile.sheets,
        };
        let name = file_name(block, status.next_revision);
        let mut about = vec![
            (
                "Block".to_string(),
                if block.is_empty() {
                    "(no block)".into()
                } else {
                    block.to_string()
                },
            ),
            ("Revision".into(), status.next_revision.to_string()),
            (
                "Generated".into(),
                chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            ),
            ("Piles".into(), status.piles.to_string()),
            ("Trackers".into(), status.trackers.to_string()),
            (
                "Ground".into(),
                "Every pile surveyed in the field; tops (ToP) optimised from those elevations."
                    .into(),
            ),
            (
                "Positions".into(),
                if settings.fixed_spans {
                    "Set-out Easting / Northing keep each torque-tube span at its design length."
                } else {
                    "Set-out positions are the design positions."
                }
                .into(),
            ),
        ];
        if let Some(last) = &status.last_export
            && last.revision < status.next_revision
        {
            about.push((
                "Replaces".into(),
                format!(
                    "revision {} of {} ({}): positions or heights changed since",
                    last.revision, last.exported_at, last.file_name
                ),
            ));
        }
        if status.ok < status.trackers {
            about.push((
                "Warning".into(),
                format!(
                    "{} of {} trackers are not compliant: see the Tracker Status column",
                    status.trackers - status.ok,
                    status.trackers
                ),
            ));
        }
        let bytes = export::workbook_with(&state, &self.imports()?, &settings, &spec, &about)?;
        Ok((name, bytes))
    }

    /// Record that a block's file was written. `content_sha` must still be the
    /// block's current one (it was not re-surveyed meanwhile). Returns the
    /// revision recorded.
    pub fn record_block_export(
        &self,
        block: &str,
        content_sha: &str,
        file_name: &str,
        by: &str,
    ) -> Result<i64> {
        let status = self
            .block_status()?
            .into_iter()
            .find(|b| b.block == block)
            .ok_or_else(|| format!("there is no block \"{block}\""))?;
        if status.content_sha.as_deref() != Some(content_sha) {
            return Err(format!(
                "block {block} changed while its file was written; it will be sent again"
            ));
        }
        if !status.needs_export {
            return Ok(status.next_revision);
        }
        self.conn
            .execute(
                "INSERT INTO block_exports (block, revision, content_sha, file_name, exported_at, exported_by, piles)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    block,
                    status.next_revision,
                    content_sha,
                    file_name,
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                    if by.trim().is_empty() { "unnamed computer" } else { by.trim() },
                    status.piles as i64
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(status.next_revision)
    }
}

pub(crate) fn mean(v: &[f64]) -> Option<f64> {
    let v: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::tests::{master, survey};

    fn status(s: &Store, block: &str) -> BlockStatus {
        s.block_status()
            .unwrap()
            .into_iter()
            .find(|b| b.block == block)
            .unwrap()
    }

    const BLOCK_A: [(&str, f64); 6] = [
        ("A1.1", 10.0),
        ("A1.2", 10.1),
        ("A1.3", 10.2),
        ("A2.1", 11.0),
        ("A2.2", 11.1),
        ("A2.3", 11.2),
    ];

    #[test]
    fn a_complete_block_is_exported_once_then_again_when_it_changes() {
        let mut s = master();
        survey(&mut s, "part.csv", &BLOCK_A[..4]);
        let a = status(&s, "A");
        assert!(!a.ready);
        assert_eq!(a.waiting, "2 piles not surveyed yet");
        survey(&mut s, "rest.csv", &BLOCK_A[4..]);
        let a = status(&s, "A");
        assert!(a.ready && a.needs_export, "{a:?}");
        assert_eq!((a.next_revision, a.current, a.trackers), (1, 2, 2));
        assert!(a.reveal_mean.is_some());
        assert!(!status(&s, "B").ready);

        let (name, bytes) = s.block_workbook("A").unwrap();
        assert_eq!(name, "Block A - pile heights rev 1.xlsx");
        use calamine::Reader;
        let mut wb: calamine::Xlsx<_> =
            calamine::open_workbook_from_rs(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(wb.sheet_names(), vec!["Piles", "About"]);
        let piles = wb.worksheet_range("Piles").unwrap();
        assert_eq!(piles.height(), 7, "header + the 6 piles of block A");
        let head: Vec<String> = piles
            .rows()
            .next()
            .unwrap()
            .iter()
            .map(|c| c.to_string())
            .collect();
        assert_eq!(head[..3], ["Block", "Tracker", "Pile ID"]);
        assert!(head.contains(&"Set-out Easting".to_string()) && head.contains(&"ToP".to_string()));

        let sha = a.content_sha.clone().unwrap();
        assert_eq!(
            s.record_block_export("A", &sha, &name, "site office")
                .unwrap(),
            1
        );
        let a = status(&s, "A");
        assert!(!a.needs_export);
        assert_eq!(a.last_export.as_ref().unwrap().exported_by, "site office");
        assert_eq!(
            s.record_block_export("A", &sha, &name, "x").unwrap(),
            1,
            "no second row"
        );
        // Re-optimising with the same elevations keeps the file current.
        s.optimize(true, "t", 1).unwrap();
        assert!(!status(&s, "A").needs_export);

        // A pile re-surveyed: a new revision is due, and replaces the first.
        survey(&mut s, "resurvey.csv", &[("A2.3", 11.6)]);
        let a = status(&s, "A");
        assert!(a.needs_export);
        assert_eq!(a.next_revision, 2);
        assert!(
            s.record_block_export("A", &sha, "old", "x").is_err(),
            "stale content"
        );
        let (name2, _) = s.block_workbook("A").unwrap();
        assert_eq!(name2, "Block A - pile heights rev 2.xlsx");
        s.record_block_export("A", a.content_sha.as_ref().unwrap(), &name2, "x")
            .unwrap();
        assert_eq!(status(&s, "A").last_export.unwrap().revision, 2);
    }

    #[test]
    fn export_profiles_are_checked_and_shared() {
        let s = master();
        assert_eq!(s.export_profiles().unwrap().len(), 2);
        assert_eq!(s.export_profile("installers").unwrap().name, "Installers");
        let mut p = s.export_profiles().unwrap();
        p.push(p[1].clone());
        assert!(
            s.set_export_profiles(&p)
                .unwrap_err()
                .contains("two export profiles")
        );
        p.pop();
        p[1].columns.push("nonsense".into());
        assert!(s.set_export_profiles(&p).is_err());
        p[1].columns.pop();
        p.push(ExportProfile {
            name: "Survey check".into(),
            columns: vec!["pile_id".into(), "ground_z".into(), "survey_offset".into()],
            sheets: vec!["piles".into()],
        });
        let r = s.revision().unwrap();
        s.set_export_profiles(&p).unwrap();
        assert_ne!(s.revision().unwrap(), r);
        assert_eq!(s.export_profile("Survey check").unwrap().columns.len(), 3);
    }

    #[test]
    fn custom_csv_has_the_chosen_columns_and_blocks() {
        let mut s = master();
        survey(&mut s, "a.csv", &BLOCK_A);
        let st = s.state().unwrap();
        let spec = ExportSpec {
            columns: vec!["pile_id".into(), "top".into(), "reveal".into()],
            blocks: vec!["A".into()],
            sheets: vec![],
        };
        let csv = export::piles_csv_with(&st, &spec).unwrap();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "Pile ID,ToP,Reveal (m)");
        assert_eq!(lines.len(), 7);
        assert!(lines[1..].iter().all(|l| l.starts_with('A')));
        let bad = ExportSpec {
            columns: vec!["top".into(), "colour".into()],
            ..Default::default()
        };
        assert!(
            export::piles_csv_with(&st, &bad)
                .unwrap_err()
                .contains("colour")
        );
        // The full export is unchanged.
        assert_eq!(
            export::piles_csv(&st)
                .lines()
                .next()
                .unwrap()
                .split(',')
                .count(),
            30
        );
    }

    #[test]
    fn summary_reports_the_mean_reveal() {
        let mut s = master();
        survey(&mut s, "a.csv", &BLOCK_A);
        let sum = s.summary().unwrap();
        assert_eq!(sum.piles_with_result, 6);
        let mean = sum.reveal_mean.unwrap();
        let st = s.state().unwrap();
        let expect = st
            .piles
            .iter()
            .filter_map(|p| p.result.as_ref().map(|r| r.reveal))
            .sum::<f64>()
            / 6.0;
        assert!((mean - expect).abs() < 1e-12);
        assert_eq!(sum.reveal_mean_survey, sum.reveal_mean);
        assert!(sum.reveal_min.unwrap() <= mean && mean <= sum.reveal_max.unwrap());
        let a = sum.blocks.iter().find(|b| b.block == "A").unwrap();
        assert!((a.reveal_mean.unwrap() - mean).abs() < 1e-12);
        assert_eq!(
            sum.blocks
                .iter()
                .find(|b| b.block == "B")
                .unwrap()
                .reveal_mean,
            None
        );
    }
}
