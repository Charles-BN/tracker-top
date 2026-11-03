//! Import reports: what was read, what was used, and why rows were skipped.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warning,
    Info,
}

/// One kind of finding, with how often it happened and a few examples.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub level: Level,
    pub message: String,
    pub count: usize,
    pub examples: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ImportReport {
    /// Database id of the import; `None` when nothing was recorded.
    pub import_id: Option<i64>,
    pub kind: String,
    pub file_name: String,
    /// True when the file was recorded in the database.
    pub accepted: bool,
    pub rows_total: usize,
    pub rows_used: usize,
    pub rows_rejected: usize,
    pub survey_date: Option<String>,
    /// Where the survey date came from: entered, column, file name, today.
    pub survey_date_source: Option<String>,
    /// Logical column -> header found in the file.
    pub columns: BTreeMap<String, String>,
    pub findings: Vec<Finding>,
}

const MAX_EXAMPLES: usize = 25;

impl ImportReport {
    pub fn new(kind: &str, file_name: &str) -> Self {
        Self {
            kind: kind.to_string(),
            file_name: file_name.to_string(),
            ..Self::default()
        }
    }

    /// Record one occurrence of a finding; occurrences with the same message are grouped.
    pub fn add(&mut self, level: Level, message: &str, example: impl Into<String>) {
        let example = example.into();
        if let Some(f) = self.findings.iter_mut().find(|f| f.message == message) {
            f.count += 1;
            if f.examples.len() < MAX_EXAMPLES && !example.is_empty() {
                f.examples.push(example);
            }
            return;
        }
        self.findings.push(Finding {
            level,
            message: message.to_string(),
            count: 1,
            examples: if example.is_empty() {
                vec![]
            } else {
                vec![example]
            },
        });
    }

    pub fn error(&mut self, message: &str) {
        self.add(Level::Error, message, "");
    }

    pub fn info(&mut self, message: &str) {
        self.add(Level::Info, message, "");
    }

    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(|f| f.level == Level::Error)
    }

    pub fn sort(&mut self) {
        self.findings.sort_by_key(|f| f.level);
    }
}
