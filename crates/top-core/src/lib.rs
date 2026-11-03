//! top-core: pile top-of-pile (ToP) height optimisation for single-axis trackers.
//!
//! Pipeline: [`csv::read_piles`] -> [`model::group_trackers`] ->
//! [`optimize::optimize_all`] -> [`csv::write_results`].
//!
//! The crate has no external dependencies. The QP solver sits behind the
//! [`qp::QpSolver`] trait so a different implementation (e.g. `clarabel`) can be
//! swapped in later.

pub mod csv;
pub mod error;
pub mod model;
pub mod optimize;
pub mod params;
pub mod qp;

pub use error::{Error, Result};
pub use model::{group_trackers, normalize_block, Pile, Tracker};
pub use optimize::{
    angle_diffs_deg, default_solver, optimize_all, optimize_tracker, PileResult, TrackerResult,
    TrackerStatus,
};
pub use params::Params;
pub use qp::{AdmmSolver, Qp, QpSolution, QpSolver, QpStatus};
