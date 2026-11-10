//! top-db: the master pile database.
//!
//! An SQLite file holds
//!
//! - the **immutable core**: the original pile list (ID, tracker, block,
//!   section, Easting, Northing, pile type, colour). Rows can be added, never
//!   changed or deleted;
//! - every imported file (master list, daily field surveys, LIDAR), with an
//!   import report. Survey and LIDAR imports can be voided but not deleted;
//! - the ground elevations read from those files (append-only);
//! - every optimisation run and its results (append-only);
//! - the files sent to the installers, block by block (append-only).
//!
//! The elevation used for a pile is its latest field survey, or its LIDAR
//! elevation when it has not been surveyed. A tracker is optimised once every
//! one of its piles has an elevation, and re-optimised whenever one of them
//! changes.

pub mod api;
pub mod blocks;
pub mod dates;
pub mod export;
pub mod gpkg;
pub mod outline;
pub mod report;
pub mod settings;
pub mod store;
pub mod sync;
pub mod table;

pub use report::{Finding, ImportReport, Level};
pub use settings::{FloodMode, Settings};
pub use store::{
    ImportRow, Kind, PileResultRow, PileState, RunSummary, State, Store, Summary, TrackerState,
    sha256_hex,
};
