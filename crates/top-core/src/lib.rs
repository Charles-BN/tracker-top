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
pub mod setout;

pub use error::{Error, Result};
pub use model::{Pile, Tracker, group_trackers, normalize_block};
pub use optimize::{
    PileResult, TrackerResult, TrackerStatus, angle_diffs_deg, default_solver, optimize_all,
    optimize_tracker,
};
pub use params::Params;
pub use qp::{AdmmSolver, Qp, QpSolution, QpSolver, QpStatus};
pub use setout::{SetOut, is_motor_pile, set_out};
