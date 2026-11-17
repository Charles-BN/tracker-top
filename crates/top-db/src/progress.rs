//! Survey progress over time, for the Plots tab: how many piles got their
//! first field-survey elevation on each day, by block.

use crate::store::{Result, Store};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::BTreeMap;

/// Piles first surveyed on one day, by block.
#[derive(Debug, Serialize, PartialEq)]
pub struct ProgressDay {
    /// `YYYY-MM-DD`: the survey date of the import (else the day it was imported).
    pub date: String,
    /// Block -> piles surveyed for the first time that day.
    pub by_block: BTreeMap<String, u32>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Progress {
    /// Block -> number of piles in the master list.
    pub block_piles: BTreeMap<String, u32>,
    /// Days with at least one newly surveyed pile, oldest first.
    pub days: Vec<ProgressDay>,
}

fn db(e: rusqlite::Error) -> String {
    format!("database: {e}")
}

pub(crate) fn survey_progress(conn: &Connection) -> Result<Progress> {
    let mut block_piles = BTreeMap::new();
    let mut st = conn
        .prepare("SELECT block, COUNT(*) FROM piles GROUP BY block")
        .map_err(db)?;
    for row in st
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))
        .map_err(db)?
    {
        let (b, n) = row.map_err(db)?;
        block_piles.insert(b, n);
    }

    // First day each pile was surveyed (voided imports do not count).
    let mut st = conn
        .prepare(
            "SELECT f.d, p.block, COUNT(*) FROM (
                 SELECT e.pile_id, MIN(COALESCE(i.survey_date, substr(i.imported_at, 1, 10))) AS d
                 FROM elevations e JOIN imports i ON i.import_id = e.import_id
                 WHERE i.kind = 'survey' AND i.voided_at IS NULL
                 GROUP BY e.pile_id
             ) f JOIN piles p ON p.pile_id = f.pile_id
             GROUP BY f.d, p.block ORDER BY f.d",
        )
        .map_err(db)?;
    let mut days: Vec<ProgressDay> = Vec::new();
    for row in st
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, u32>(2)?))
        })
        .map_err(db)?
    {
        let (d, b, n) = row.map_err(db)?;
        if days.last().is_none_or(|x| x.date != d) {
            days.push(ProgressDay { date: d, by_block: BTreeMap::new() });
        }
        days.last_mut().unwrap().by_block.insert(b, n);
    }
    Ok(Progress { block_piles, days })
}

impl Store {
    /// Piles surveyed for the first time, day by day and block by block.
    pub fn survey_progress(&self) -> Result<Progress> {
        survey_progress(&self.conn)
    }
}
