//! One database shared by several computers through a shared folder (Google
//! Drive for desktop, OneDrive, a network drive) or Google Drive online.
//!
//! There is no server: every computer works on its own copy and exchanges the
//! whole database file with the shared folder. The page does the file transfers
//! (it knows the folder); this module decides what to do with them.
//!
//! * `revision` (in `meta`) is a random value that changes, through triggers in
//!   schema.sql, whenever something worth sharing changes: an import, a void, an
//!   optimisation run, settings, export profiles, an installer export.
//! * `sync_base` is the revision of the shared copy this computer last took or
//!   wrote. `revision != sync_base` means this computer has changes the shared
//!   copy does not have yet ("dirty").
//!
//! When a shared copy arrives (`incoming`):
//! * same revision as `sync_base`: nothing new;
//! * this computer has no changes of its own: take the shared copy (`adopt`);
//! * both changed: copy this computer's new imports, voids, settings and export
//!   log into the shared copy, re-optimise, and use the result (`merge`). The
//!   page then writes it back to the shared folder.
//!
//! Imports are never edited or deleted, so merging is safe: it is a union. The
//! only real conflict is two different master pile lists, which is refused.

use crate::store::{Result, Store};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// Settings of this computer only: kept when a shared copy is taken.
const LOCAL_KEYS: [&str; 2] = ["watch_folder", "sync_base"];

/// Shared settings and when they were last changed: the newest wins in a merge.
const SHARED_KEYS: [(&str, &str); 2] = [
    ("settings", "settings_at"),
    ("export_profiles", "export_profiles_at"),
];

/// A new random revision (16 hex digits), from SQLite's random source so that
/// it works the same in the browser.
pub fn new_revision() -> String {
    let c = rusqlite::Connection::open_in_memory().expect("in-memory database");
    c.query_row("SELECT lower(hex(randomblob(8)))", [], |r| r.get(0))
        .expect("random value")
}

/// Current time, UTC, sortable as text.
pub fn utc_now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[derive(Clone, Debug, Serialize)]
pub struct SyncStatus {
    pub revision: String,
    /// Revision of the shared copy this computer is in step with.
    pub base: Option<String>,
    /// This computer has changes the shared copy does not have.
    pub dirty: bool,
    /// Who wrote this copy to the shared folder last, and when (UTC).
    pub saved_by: Option<String>,
    pub saved_at: Option<String>,
    pub imports: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SyncReport {
    /// "none", "adopted", "merged" or "conflict".
    pub action: String,
    /// Imports of this computer added to the shared copy.
    pub imports_added: usize,
    /// Imports voided here and not yet in the shared copy.
    pub voids_applied: usize,
    /// Files already in the shared copy (imported on both computers).
    pub duplicates_skipped: usize,
    /// Installer exports of this computer added to the shared log.
    pub exports_added: usize,
    /// Shared settings taken from this computer because they are newer.
    pub settings_kept: Vec<String>,
    /// Who saved the shared copy last.
    pub saved_by: Option<String>,
    pub saved_at: Option<String>,
    pub message: String,
}

/// An import row, everything but its id.
struct ImportCopy {
    id: i64,
    kind: String,
    file_name: String,
    sha256: String,
    imported_at: String,
    survey_date: Option<String>,
    origin: String,
    rows_total: i64,
    rows_used: i64,
    rows_rejected: i64,
    report: String,
    voided_at: Option<String>,
    void_reason: Option<String>,
}

type PileRow = (
    String,
    String,
    String,
    String,
    f64,
    f64,
    String,
    String,
    String,
);

impl Store {
    pub fn revision(&self) -> Result<String> {
        Ok(self.meta("revision")?.unwrap_or_default())
    }

    fn import_count(&self) -> Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get::<_, i64>(0))
            .map(|n| n as usize)
            .map_err(|e| e.to_string())
    }

    pub fn sync_status(&self) -> Result<SyncStatus> {
        let revision = self.revision()?;
        let base = self.meta("sync_base")?;
        Ok(SyncStatus {
            dirty: base.as_deref() != Some(revision.as_str()),
            revision,
            base,
            saved_by: self.meta("saved_by")?,
            saved_at: self.meta("saved_at")?,
            imports: self.import_count()?,
        })
    }

    /// The database to write to the shared folder, marked as saved by `by`.
    /// Call `mark_saved` once it is written.
    pub fn snapshot(&mut self, by: &str) -> Result<Vec<u8>> {
        let by = by.trim();
        self.set_meta(
            "saved_by",
            if by.is_empty() {
                "unnamed computer"
            } else {
                by
            },
        )?;
        self.set_meta("saved_at", &utc_now())?;
        self.snapshot_revision = Some(self.revision()?);
        self.to_bytes()
    }

    /// The last snapshot is now the shared copy: this computer is in step with
    /// it (unless something changed here since the snapshot).
    pub fn mark_saved(&mut self) -> Result<()> {
        let rev = self
            .snapshot_revision
            .take()
            .ok_or("no copy was prepared for the shared folder")?;
        self.set_meta("sync_base", &rev)
    }

    /// A copy of the shared database arrived: bring this one in step with it.
    pub fn incoming(&mut self, shared: &[u8], by: &str, threads: usize) -> Result<SyncReport> {
        let mut theirs = Store::from_bytes(shared)
            .map_err(|e| format!("the shared database cannot be opened: {e}"))?;
        let their_rev = theirs.revision()?;
        let mut report = SyncReport {
            saved_by: theirs.meta("saved_by")?,
            saved_at: theirs.meta("saved_at")?,
            ..Default::default()
        };
        let status = self.sync_status()?;
        if status.base.as_deref() == Some(their_rev.as_str()) {
            report.action = "none".into();
            report.message = "Already in step with the shared database.".into();
            return Ok(report);
        }
        if !status.dirty || status.imports == 0 {
            self.adopt_store(&mut theirs, &their_rev, threads)?;
            report.action = "adopted".into();
            report.message = format!(
                "Updated from the shared database{}.",
                saved_by_text(&report)
            );
            return Ok(report);
        }
        match self.merge_into(&mut theirs, &mut report) {
            Ok(()) => {}
            Err(e) => {
                report.action = "conflict".into();
                report.message = e;
                return Ok(report);
            }
        }
        let label = if by.trim().is_empty() {
            "another computer"
        } else {
            by.trim()
        };
        theirs.optimize(false, &format!("changes merged from {label}"), threads)?;
        self.replace_with(&mut theirs)?;
        self.set_meta("sync_base", &their_rev)?;
        report.action = "merged".into();
        report.message = format!(
            "Merged with the shared database{}: {} import{} from this computer added{}.",
            saved_by_text(&report),
            report.imports_added,
            if report.imports_added == 1 { "" } else { "s" },
            if report.duplicates_skipped > 0 {
                format!(", {} already there", report.duplicates_skipped)
            } else {
                String::new()
            }
        );
        Ok(report)
    }

    /// Replace this database with the shared copy, whatever changed here (used
    /// to settle a conflict once this computer's copy is kept aside).
    pub fn adopt(&mut self, shared: &[u8], threads: usize) -> Result<()> {
        let mut theirs = Store::from_bytes(shared)?;
        let rev = theirs.revision()?;
        self.adopt_store(&mut theirs, &rev, threads)
    }

    fn adopt_store(&mut self, theirs: &mut Store, their_rev: &str, threads: usize) -> Result<()> {
        self.replace_with(theirs)?;
        self.set_meta("sync_base", their_rev)?;
        // Results made by another version of the program are brought up to date
        // here; that is a change of this computer's, shared on the next save.
        self.optimize(false, "program update", threads)?;
        Ok(())
    }

    /// Make this database an exact copy of `other`, keeping this computer's own
    /// settings (watch folder, sync state).
    fn replace_with(&mut self, other: &mut Store) -> Result<()> {
        let mut keep = Vec::new();
        for k in LOCAL_KEYS {
            keep.push((k, self.meta(k)?));
        }
        let on_disk = self.conn.path().is_some_and(|p| !p.is_empty());
        if on_disk {
            // Copy page by page into the file. A file in WAL mode cannot take
            // a copy with another page size (the browser version's SQLite uses
            // 8 KB pages, the desktop's 4 KB): leave WAL mode for the copy.
            let mode: String = self
                .conn
                .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            let wal = mode.eq_ignore_ascii_case("wal");
            if wal {
                self.conn
                    .pragma_update(None, "journal_mode", "DELETE")
                    .map_err(|e| e.to_string())?;
            }
            let copied = (|| {
                let backup = rusqlite::backup::Backup::new(&other.conn, &mut self.conn)?;
                while backup.step(-1)? != rusqlite::backup::StepResult::Done {}
                Ok::<(), rusqlite::Error>(())
            })();
            if wal {
                self.conn
                    .pragma_update(None, "journal_mode", "WAL")
                    .map_err(|e| e.to_string())?;
            }
            copied.map_err(|e| format!("cannot copy the database: {e}"))?;
        } else {
            // In memory (browser version): load the other database's bytes
            // whole, whatever their page size.
            let bytes = other.to_bytes()?;
            self.load_bytes(&bytes)?;
        }
        for (k, v) in keep {
            match v {
                Some(v) => self.set_meta(k, &v)?,
                None => {
                    self.conn
                        .execute("DELETE FROM meta WHERE key = ?1", [k])
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        self.snapshot_revision = None;
        Ok(())
    }

    fn import_copies(&self) -> Result<Vec<ImportCopy>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT import_id, kind, file_name, sha256, imported_at, survey_date, origin, rows_total,
                        rows_used, rows_rejected, report, voided_at, void_reason
                 FROM imports ORDER BY import_id",
            )
            .map_err(|e| e.to_string())?;
        st.query_map([], |r| {
            Ok(ImportCopy {
                id: r.get(0)?,
                kind: r.get(1)?,
                file_name: r.get(2)?,
                sha256: r.get(3)?,
                imported_at: r.get(4)?,
                survey_date: r.get(5)?,
                origin: r.get(6)?,
                rows_total: r.get(7)?,
                rows_used: r.get(8)?,
                rows_rejected: r.get(9)?,
                report: r.get(10)?,
                voided_at: r.get(11)?,
                void_reason: r.get(12)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())
    }

    fn piles_of(&self, import_id: i64) -> Result<Vec<PileRow>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT pile_id, tracker, block, section, easting, northing, pile_type, color, tracker_type
                 FROM piles WHERE import_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        st.query_map([import_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())
    }

    /// Add what this database has and `theirs` lacks to `theirs`.
    fn merge_into(&self, theirs: &mut Store, report: &mut SyncReport) -> Result<()> {
        let err = |e: rusqlite::Error| e.to_string();
        let mine = self.import_copies()?;
        let their_imports = theirs.import_copies()?;
        let by_identity: HashMap<(String, String, String), (i64, bool)> = their_imports
            .iter()
            .map(|i| {
                (
                    (i.kind.clone(), i.sha256.clone(), i.imported_at.clone()),
                    (i.id, i.voided_at.is_some()),
                )
            })
            .collect();
        let mut active: HashSet<(String, String)> = their_imports
            .iter()
            .filter(|i| i.voided_at.is_none())
            .map(|i| (i.kind.clone(), i.sha256.clone()))
            .collect();

        // Master piles must agree: the pile list is the immutable core.
        let their_piles: HashMap<String, PileRow> = {
            let mut st = theirs
                .conn
                .prepare(
                    "SELECT pile_id, tracker, block, section, easting, northing, pile_type, color, tracker_type FROM piles",
                )
                .map_err(err)?;
            st.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ),
                ))
            })
            .map_err(err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(err)?
        };

        let tx = theirs.conn.transaction().map_err(err)?;
        for imp in &mine {
            let key = (
                imp.kind.clone(),
                imp.sha256.clone(),
                imp.imported_at.clone(),
            );
            if let Some(&(their_id, their_void)) = by_identity.get(&key) {
                if let (Some(at), false) = (&imp.voided_at, their_void) {
                    tx.execute(
                        "UPDATE imports SET voided_at = ?1, void_reason = ?2 WHERE import_id = ?3",
                        params![at, imp.void_reason, their_id],
                    )
                    .map_err(err)?;
                    report.voids_applied += 1;
                }
                continue;
            }
            if imp.voided_at.is_none() && active.contains(&(imp.kind.clone(), imp.sha256.clone())) {
                report.duplicates_skipped += 1;
                continue;
            }
            let piles = if imp.kind == "master" {
                let piles = self.piles_of(imp.id)?;
                if let Some(p) = piles
                    .iter()
                    .find(|p| their_piles.get(&p.0).is_some_and(|t| !same_pile(t, p)))
                {
                    return Err(format!(
                        "The shared database has a different master pile list (pile {} differs). \
                         The two databases cannot be merged: choose which one to keep.",
                        p.0
                    ));
                }
                piles
                    .into_iter()
                    .filter(|p| !their_piles.contains_key(&p.0))
                    .collect()
            } else {
                Vec::new()
            };
            tx.execute(
                "INSERT INTO imports (kind, file_name, sha256, imported_at, survey_date, origin, rows_total,
                                      rows_used, rows_rejected, report, voided_at, void_reason)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    imp.kind,
                    imp.file_name,
                    imp.sha256,
                    imp.imported_at,
                    imp.survey_date,
                    imp.origin,
                    imp.rows_total,
                    imp.rows_used,
                    imp.rows_rejected,
                    imp.report,
                    imp.voided_at,
                    imp.void_reason
                ],
            )
            .map_err(err)?;
            let new_id = tx.last_insert_rowid();
            for p in piles {
                tx.execute(
                    "INSERT INTO piles (pile_id, tracker, block, section, easting, northing, pile_type, color, tracker_type, import_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![p.0, p.1, p.2, p.3, p.4, p.5, p.6, p.7, p.8, new_id],
                )
                .map_err(err)?;
            }
            // Elevations and flood values of the import, under its new number.
            let mut st = self
                .conn
                .prepare(
                    "SELECT pile_id, z, easting, northing, offset_m, matched_by, code FROM elevations WHERE import_id = ?1",
                )
                .map_err(err)?;
            let rows = st
                .query_map([imp.id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, f64>(1)?,
                        r.get::<_, Option<f64>>(2)?,
                        r.get::<_, Option<f64>>(3)?,
                        r.get::<_, Option<f64>>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                    ))
                })
                .map_err(err)?;
            for row in rows {
                let (pile, z, e, n, off, by, code) = row.map_err(err)?;
                tx.execute(
                    "INSERT INTO elevations (import_id, pile_id, z, easting, northing, offset_m, matched_by, code)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![new_id, pile, z, e, n, off, by, code],
                )
                .map_err(|e| format!("cannot merge {}: {e}", imp.file_name))?;
            }
            let mut st = self
                .conn
                .prepare("SELECT pile_id, value, matched_by FROM flood_values WHERE import_id = ?1")
                .map_err(err)?;
            let rows = st
                .query_map([imp.id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, f64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(err)?;
            for row in rows {
                let (pile, v, by) = row.map_err(err)?;
                tx.execute(
                    "INSERT INTO flood_values (import_id, pile_id, value, matched_by) VALUES (?1, ?2, ?3, ?4)",
                    params![new_id, pile, v, by],
                )
                .map_err(|e| format!("cannot merge {}: {e}", imp.file_name))?;
            }
            if imp.voided_at.is_none() {
                active.insert((imp.kind.clone(), imp.sha256.clone()));
            }
            report.imports_added += 1;
        }

        // Installer export log: a union.
        let seen: HashSet<(String, String)> = {
            let mut st = tx
                .prepare("SELECT block, content_sha FROM block_exports")
                .map_err(err)?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(err)?
                .collect::<rusqlite::Result<_>>()
                .map_err(err)?
        };
        let mut st = self
            .conn
            .prepare(
                "SELECT block, revision, content_sha, file_name, exported_at, exported_by, piles
                 FROM block_exports ORDER BY export_id",
            )
            .map_err(err)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .map_err(err)?;
        for row in rows {
            let (block, rev, sha, file, at, by, piles) = row.map_err(err)?;
            if seen.contains(&(block.clone(), sha.clone())) {
                continue;
            }
            tx.execute(
                "INSERT INTO block_exports (block, revision, content_sha, file_name, exported_at, exported_by, piles)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![block, rev, sha, file, at, by, piles],
            )
            .map_err(err)?;
            report.exports_added += 1;
        }

        // Shared settings: the most recently changed wins.
        for (key, at_key) in SHARED_KEYS {
            let get = |c: &rusqlite::Connection, k: &str| -> Result<Option<String>> {
                c.query_row("SELECT value FROM meta WHERE key = ?1", [k], |r| r.get(0))
                    .optional()
                    .map_err(|e| e.to_string())
            };
            let (Some(value), Some(mine_at)) = (get(&self.conn, key)?, get(&self.conn, at_key)?)
            else {
                continue;
            };
            let theirs_at = get(&tx, at_key)?.unwrap_or_default();
            if mine_at > theirs_at && get(&tx, key)?.as_deref() != Some(value.as_str()) {
                for (k, v) in [(key, value.as_str()), (at_key, mine_at.as_str())] {
                    tx.execute(
                        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
                        params![k, v],
                    )
                    .map_err(err)?;
                }
                report.settings_kept.push(key.to_string());
            }
        }
        tx.commit().map_err(err)
    }
}

fn same_pile(a: &PileRow, b: &PileRow) -> bool {
    a.1 == b.1 && (a.4 - b.4).abs() < 1e-6 && (a.5 - b.5).abs() < 1e-6
}

fn saved_by_text(r: &SyncReport) -> String {
    match (&r.saved_by, &r.saved_at) {
        (Some(by), Some(at)) => format!(" (saved by {by}, {at})"),
        (Some(by), None) => format!(" (saved by {by})"),
        _ => String::new(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::Settings;
    use crate::store::Kind;

    /// Two blocks: A (trackers A1, A2) and B (tracker B1), three piles each.
    pub const MASTER: &str = "Pile ID,Block,Tracker,Easting,Northing,Pile Type\n\
        A1.1,A,A1,0,0,X\nA1.2,A,A1,0,8,X\nA1.3,A,A1,0,16,X\n\
        A2.1,A,A2,10,0,X\nA2.2,A,A2,10,8,X\nA2.3,A,A2,10,16,X\n\
        B1.1,B,B1,50,0,X\nB1.2,B,B1,50,8,X\nB1.3,B,B1,50,16,X\n";

    pub fn master() -> Store {
        let mut s = Store::open_in_memory().unwrap();
        assert!(
            s.import_file(Kind::Master, "m.csv", MASTER.as_bytes(), None, "t")
                .unwrap()
                .accepted
        );
        s
    }

    /// A survey file of `(pile, z)` at the design positions.
    pub fn survey_file(rows: &[(&str, f64)]) -> String {
        rows.iter()
            .map(|(id, z)| {
                let line = MASTER
                    .lines()
                    .find(|l| l.starts_with(&format!("{id},")))
                    .unwrap();
                let f: Vec<&str> = line.split(',').collect();
                format!("{id},{},{},{z}\n", f[3], f[4])
            })
            .collect()
    }

    pub fn survey(s: &mut Store, name: &str, rows: &[(&str, f64)]) -> i64 {
        let rep = s
            .import_file(
                Kind::Survey,
                name,
                survey_file(rows).as_bytes(),
                Some("2026-11-06"),
                "t",
            )
            .unwrap();
        assert!(rep.accepted, "{:?}", rep.findings);
        s.optimize(false, name, 1).unwrap();
        rep.import_id.unwrap()
    }

    /// Write `s` to the "shared folder" (bytes), as the page does.
    fn save(s: &mut Store, by: &str) -> Vec<u8> {
        let bytes = s.snapshot(by).unwrap();
        s.mark_saved().unwrap();
        bytes
    }

    fn files(s: &Store) -> Vec<(String, bool)> {
        let mut v: Vec<_> = s
            .imports()
            .unwrap()
            .into_iter()
            .map(|i| (i.file_name, i.voided_at.is_some()))
            .collect();
        v.sort();
        v
    }

    fn tops(s: &Store) -> Vec<(String, Option<i64>)> {
        s.state()
            .unwrap()
            .piles
            .into_iter()
            .map(|p| (p.pile_id, p.result.map(|r| (r.top * 1000.0).round() as i64)))
            .collect()
    }

    #[test]
    fn revision_changes_only_for_shared_changes() {
        let s = Store::open_in_memory().unwrap();
        let r0 = s.revision().unwrap();
        assert_eq!(r0.len(), 16);
        s.set_watch_folder(Some("C:\\survey")).unwrap();
        assert_eq!(
            s.revision().unwrap(),
            r0,
            "the watch folder is this computer's own"
        );
        s.set_settings(&Settings::default()).unwrap();
        let r1 = s.revision().unwrap();
        assert_ne!(r1, r0);
        s.set_settings(&Settings::default()).unwrap();
        assert_eq!(
            s.revision().unwrap(),
            r1,
            "saving unchanged settings changes nothing"
        );

        let mut s = master();
        assert!(s.sync_status().unwrap().dirty);
        let _ = save(&mut s, "PC1");
        let st = s.sync_status().unwrap();
        assert!(!st.dirty);
        assert_eq!(st.saved_by.as_deref(), Some("PC1"));
        s.set_watch_folder(Some("D:\\x")).unwrap();
        assert!(!s.sync_status().unwrap().dirty);
        survey(&mut s, "s1.csv", &[("A1.1", 10.0)]);
        assert!(s.sync_status().unwrap().dirty);
        // Saving needs a prepared copy.
        let mut fresh = Store::open_in_memory().unwrap();
        assert!(fresh.mark_saved().is_err());
    }

    #[test]
    fn a_new_computer_takes_the_shared_copy() {
        let mut a = master();
        survey(
            &mut a,
            "s1.csv",
            &[("A1.1", 10.0), ("A1.2", 10.1), ("A1.3", 10.2)],
        );
        let shared = save(&mut a, "site office");

        let mut b = Store::open_in_memory().unwrap();
        b.set_watch_folder(Some("E:\\b")).unwrap();
        let rep = b.incoming(&shared, "laptop", 1).unwrap();
        assert_eq!(rep.action, "adopted", "{}", rep.message);
        assert!(rep.message.contains("site office"), "{}", rep.message);
        assert_eq!(files(&b), files(&a));
        assert_eq!(tops(&b), tops(&a));
        assert!(!b.sync_status().unwrap().dirty);
        assert_eq!(b.watch_folder().unwrap().as_deref(), Some("E:\\b"));
        assert_eq!(b.incoming(&shared, "laptop", 1).unwrap().action, "none");
    }

    #[test]
    fn changes_on_two_computers_are_merged() {
        let mut a = master();
        let s1 = survey(
            &mut a,
            "s1.csv",
            &[("A1.1", 10.0), ("A1.2", 10.1), ("A1.3", 10.2)],
        );
        let shared1 = save(&mut a, "A");
        let mut b = Store::open_in_memory().unwrap();
        assert_eq!(b.incoming(&shared1, "B", 1).unwrap().action, "adopted");

        // A: a new survey, saved to the shared folder.
        survey(
            &mut a,
            "s2.csv",
            &[("A2.1", 11.0), ("A2.2", 11.1), ("A2.3", 11.3)],
        );
        let shared2 = save(&mut a, "A");
        // B, at the same time: another survey, a void and new settings.
        survey(
            &mut b,
            "s3.csv",
            &[("B1.1", 12.0), ("B1.2", 12.0), ("B1.3", 12.1)],
        );
        b.void_import(s1, "wrong instrument height").unwrap();
        let mut set = b.settings().unwrap();
        set.reveal_nominal = 1.45;
        set.auto_export_folder = "Installers/Heights".into();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        b.set_settings(&set).unwrap();
        b.optimize(false, "t", 1).unwrap();

        let rep = b.incoming(&shared2, "B", 1).unwrap();
        assert_eq!(rep.action, "merged", "{}", rep.message);
        assert_eq!((rep.imports_added, rep.voids_applied), (1, 1));
        assert_eq!(rep.settings_kept, vec!["settings".to_string()]);
        assert!(
            b.sync_status().unwrap().dirty,
            "the merged copy goes back to the folder"
        );
        let shared3 = save(&mut b, "B");

        // A had no change of its own since its save: it takes the merged copy.
        assert_eq!(a.incoming(&shared3, "A", 1).unwrap().action, "adopted");
        assert_eq!(files(&a), files(&b));
        assert_eq!(
            files(&a),
            vec![
                ("m.csv".into(), false),
                ("s1.csv".into(), true),
                ("s2.csv".into(), false),
                ("s3.csv".into(), false)
            ]
        );
        assert_eq!(a.settings().unwrap().reveal_nominal, 1.45);
        assert_eq!(tops(&a), tops(&b));
        let st = a.state().unwrap();
        let p = st.piles.iter().find(|p| p.pile_id == "B1.3").unwrap();
        assert_eq!(p.z, Some(12.1));
        let p = st.piles.iter().find(|p| p.pile_id == "A1.1").unwrap();
        assert_eq!(p.z_source, None, "s1 is void everywhere");
        assert!(
            st.trackers
                .iter()
                .filter(|t| t.n_missing == 0)
                .all(|t| !t.stale)
        );
    }

    #[test]
    fn the_same_file_imported_on_both_computers_is_kept_once() {
        let mut a = master();
        let shared0 = save(&mut a, "A");
        let mut b = Store::open_in_memory().unwrap();
        b.incoming(&shared0, "B", 1).unwrap();
        let rows = [("B1.1", 9.0), ("B1.2", 9.1), ("B1.3", 9.2)];
        survey(&mut a, "same.csv", &rows);
        survey(&mut b, "same.csv", &rows);
        let shared1 = save(&mut a, "A");
        let rep = b.incoming(&shared1, "B", 1).unwrap();
        assert_eq!(rep.action, "merged");
        // Recognised as the same import (same file, same time) or as a second
        // copy of a file already there: either way it is not added twice.
        assert_eq!(rep.imports_added, 0);
        assert_eq!(files(&b).len(), 2);
    }

    #[test]
    fn two_different_master_lists_are_not_merged() {
        let mut a = master();
        let shared = save(&mut a, "A");
        let mut b = Store::open_in_memory().unwrap();
        let other = MASTER.replace("A1.2,A,A1,0,8,X", "A1.2,A,A1,0,9,X");
        b.import_file(Kind::Master, "other.csv", other.as_bytes(), None, "t")
            .unwrap();
        let rep = b.incoming(&shared, "B", 1).unwrap();
        assert_eq!(rep.action, "conflict");
        assert!(rep.message.contains("A1.2"), "{}", rep.message);
        assert_eq!(
            files(&b),
            vec![("other.csv".into(), false)],
            "nothing changed here"
        );
        // Settled by taking the shared one.
        b.adopt(&shared, 1).unwrap();
        assert_eq!(files(&b), vec![("m.csv".into(), false)]);
        assert!(!b.sync_status().unwrap().dirty);
    }

    /// The bytes of a copy of `s` with another page size.
    fn with_page_size(s: &mut Store, size: u32) -> Store {
        let path =
            std::env::temp_dir().join(format!("tracker-top-ps-{}-{size}.db", std::process::id()));
        std::fs::write(&path, s.to_bytes().unwrap()).unwrap();
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(&format!(
            "PRAGMA journal_mode = DELETE; PRAGMA page_size = {size}; VACUUM;"
        ))
        .unwrap();
        drop(c);
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        Store::from_bytes(&bytes).unwrap()
    }

    #[test]
    fn a_database_file_on_disk_takes_the_shared_copy() {
        let mut a = master();
        survey(
            &mut a,
            "s1.csv",
            &[("A1.1", 10.0), ("A1.2", 10.1), ("A1.3", 10.2)],
        );
        // Made by the browser version: 8 KB pages, where the file uses 4 KB.
        let mut a = with_page_size(&mut a, 8192);
        let shared = save(&mut a, "A");
        assert_eq!(u16::from_be_bytes([shared[16], shared[17]]), 8192);
        let path = std::env::temp_dir().join(format!("tracker-top-sync-{}.db", std::process::id()));
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
        {
            let mut d = Store::open(&path).unwrap();
            d.set_watch_folder(Some("C:\\desktop")).unwrap();
            assert_eq!(d.incoming(&shared, "desktop", 1).unwrap().action, "adopted");
        }
        let mut d = Store::open(&path).unwrap();
        assert_eq!(files(&d), files(&a));
        assert_eq!(tops(&d), tops(&a));
        assert_eq!(d.watch_folder().unwrap().as_deref(), Some("C:\\desktop"));
        assert!(!d.sync_status().unwrap().dirty);
        let mode: String = d
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal", "the file is back in WAL mode");

        // And back: the file's copy (4 KB pages) goes into a browser-made one (8 KB).
        survey(
            &mut d,
            "s2.csv",
            &[("B1.1", 9.0), ("B1.2", 9.1), ("B1.3", 9.2)],
        );
        let from_disk = save(&mut d, "desktop");
        let mut a2 = Store::from_bytes(&shared).unwrap();
        let rep = a2.incoming(&from_disk, "A", 1).unwrap();
        assert!(
            rep.action == "adopted" || rep.action == "merged",
            "{}",
            rep.message
        );
        assert_eq!(files(&a2), files(&d));
        assert_eq!(tops(&a2), tops(&d));
        drop(d);
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
    }
}
