//! Aligning neighbouring rows.
//!
//! In the early morning and late evening each tracker row can shade the row
//! behind it; backtracking flattens the modules to avoid this, from the height
//! it expects the neighbouring rows to be at. A steady slope across the rows is
//! fine (the tracker controller can allow for it), but a row that sits off the
//! line through its two neighbours, or whose height step to a neighbour changes
//! along the row, gets shade the backtracking cannot avoid.
//!
//! With alignment on, each tracker's optimisation gets one more term pulling
//! every top towards a target taken from the neighbouring rows at the same
//! Northing:
//!
//! * between two neighbours, the line through them: `(top_west + top_east) / 2`;
//! * beside one neighbour only (edge rows, row ends), that neighbour's profile
//!   moved to this row's own mean height difference: the rows run parallel.
//!
//! The target is first brought inside the pile's reveal window (and above its
//! lowest top), so the pull never asks for cut/fill; every other limit stays a
//! hard constraint. Because each row's target depends on its neighbours, rows
//! are solved again in a few rounds ([`align_all`]), each round from the tops
//! of the round before, until nothing moves by more than 5 mm.
//!
//! Rows are tracker columns along the Northing: neighbours are the trackers one
//! row pitch to the east and to the west (the pitch is the usual distance
//! between neighbouring trackers, found from the data) that cover the same
//! Northing. A gap wider than 1.25 pitches (a road) breaks the neighbourhood.

use crate::model::Tracker;
use crate::optimize::{TrackerResult, optimize_tracker_aligned, par_map};
use crate::params::Params;
use crate::qp::QpSolver;
use std::collections::BTreeSet;

/// Rows within this distance (m) of a neighbour's end piles still use it.
const END_MARGIN: f64 = 1.0;
/// Rows whose tops moved by less than this (m) in a round are settled.
pub const SETTLED: f64 = 0.005;

/// Where the trackers are, to find neighbouring rows.
#[derive(Clone, Debug)]
pub struct Rows {
    /// Mean Easting of each tracker (its axis).
    axis: Vec<f64>,
    /// Northing of each pile, per tracker, in the tracker's pile order.
    north: Vec<Vec<f64>>,
    /// Pile indices of each tracker sorted by Northing.
    order: Vec<Vec<usize>>,
    /// Lowest and highest Northing of each tracker.
    span: Vec<(f64, f64)>,
    /// Trackers sorted by axis.
    by_axis: Vec<usize>,
    /// Usual distance between neighbouring rows (m); `None` with fewer than two rows.
    pub pitch: Option<f64>,
}

impl Rows {
    /// From the (Easting, Northing) of every pile of every tracker.
    pub fn new(positions: &[Vec<(f64, f64)>]) -> Self {
        let n = positions.len();
        let mut axis = Vec::with_capacity(n);
        let mut north = Vec::with_capacity(n);
        let mut order = Vec::with_capacity(n);
        let mut span = Vec::with_capacity(n);
        for pos in positions {
            let m = pos.len().max(1) as f64;
            axis.push(pos.iter().map(|p| p.0).sum::<f64>() / m);
            let ns: Vec<f64> = pos.iter().map(|p| p.1).collect();
            let mut o: Vec<usize> = (0..ns.len()).collect();
            o.sort_by(|&a, &b| ns[a].total_cmp(&ns[b]));
            span.push(match (o.first(), o.last()) {
                (Some(&a), Some(&b)) => (ns[a], ns[b]),
                _ => (f64::NAN, f64::NAN),
            });
            north.push(ns);
            order.push(o);
        }
        let mut by_axis: Vec<usize> = (0..n).filter(|&i| axis[i].is_finite()).collect();
        by_axis.sort_by(|&a, &b| axis[a].total_cmp(&axis[b]));
        let mut rows = Rows {
            axis,
            north,
            order,
            span,
            by_axis,
            pitch: None,
        };
        rows.pitch = rows.find_pitch();
        rows
    }

    fn overlap(&self, a: usize, b: usize) -> f64 {
        let (a0, a1) = self.span[a];
        let (b0, b1) = self.span[b];
        a1.min(b1) - a0.max(b0)
    }

    /// Median distance from each tracker to the nearest tracker east of it
    /// that runs beside it.
    fn find_pitch(&self) -> Option<f64> {
        let mut gaps = Vec::new();
        for (k, &i) in self.by_axis.iter().enumerate() {
            let mut best = f64::INFINITY;
            for &j in &self.by_axis[k + 1..] {
                let dx = self.axis[j] - self.axis[i];
                if dx > 30.0 || dx > best {
                    break;
                }
                if dx > 1.0 && self.overlap(i, j) > 0.0 {
                    best = dx;
                }
            }
            if best.is_finite() {
                gaps.push(best);
            }
        }
        if gaps.is_empty() {
            return None;
        }
        gaps.sort_by(f64::total_cmp);
        Some(gaps[gaps.len() / 2])
    }

    /// Trackers one row east (`side` = 1) or west (-1) of tracker `i` that run beside it.
    fn column(&self, i: usize, side: f64) -> Vec<usize> {
        let Some(pitch) = self.pitch else {
            return Vec::new();
        };
        let want = self.axis[i] + side * pitch;
        let tol = 0.25 * pitch;
        let start = self
            .by_axis
            .partition_point(|&j| self.axis[j] < want - tol);
        self.by_axis[start..]
            .iter()
            .copied()
            .take_while(|&j| self.axis[j] <= want + tol)
            .filter(|&j| j != i && self.overlap(i, j) > -END_MARGIN)
            .collect()
    }

    /// Trackers in the rows on either side of tracker `i`.
    pub fn neighbours(&self, i: usize) -> Vec<usize> {
        let mut v = self.column(i, -1.0);
        v.extend(self.column(i, 1.0));
        v
    }

    /// Top of tracker `j` at Northing `y` (linear between its piles), if `y`
    /// is along it.
    fn top_at(&self, j: usize, tops: &[f64], y: f64) -> Option<f64> {
        let (n0, n1) = self.span[j];
        if !(y >= n0 - END_MARGIN && y <= n1 + END_MARGIN) || tops.len() != self.north[j].len() {
            return None;
        }
        let o = &self.order[j];
        let ns = &self.north[j];
        let k = o.partition_point(|&p| ns[p] < y);
        if k == 0 {
            return Some(tops[o[0]]);
        }
        if k == o.len() {
            return Some(tops[o[o.len() - 1]]);
        }
        let (a, b) = (o[k - 1], o[k]);
        let f = if ns[b] > ns[a] {
            (y - ns[a]) / (ns[b] - ns[a])
        } else {
            0.0
        };
        Some(tops[a] + f * (tops[b] - tops[a]))
    }

    /// Height of the neighbouring row on one side at each pile of tracker `i`.
    fn side_heights(&self, i: usize, side: f64, tops: &[Option<Vec<f64>>]) -> Vec<Option<f64>> {
        let cols = self.column(i, side);
        self.north[i]
            .iter()
            .map(|&y| {
                cols.iter()
                    .find_map(|&j| tops[j].as_deref().and_then(|t| self.top_at(j, t, y)))
            })
            .collect()
    }

    /// Target top of each pile of tracker `i`, from the current tops of the
    /// others (`None` where a tracker has no tops). `None` for a pile with no
    /// neighbouring row beside it.
    pub fn targets(&self, i: usize, tops: &[Option<Vec<f64>>]) -> Vec<Option<f64>> {
        let west = self.side_heights(i, -1.0, tops);
        let east = self.side_heights(i, 1.0, tops);
        let own = tops[i].as_deref();
        // Mean height of this row above one neighbour, where that one is the only one.
        let offset = |side: &[Option<f64>]| -> Option<f64> {
            let own = own?;
            let d: Vec<f64> = side
                .iter()
                .zip(own)
                .filter_map(|(s, o)| s.map(|s| o - s))
                .collect();
            (!d.is_empty()).then(|| d.iter().sum::<f64>() / d.len() as f64)
        };
        let (dw, de) = (offset(&west), offset(&east));
        west.iter()
            .zip(&east)
            .map(|(w, e)| match (w, e) {
                (Some(w), Some(e)) => Some((w + e) / 2.0),
                (Some(w), None) => dw.map(|d| w + d),
                (None, Some(e)) => de.map(|d| e + d),
                (None, None) => None,
            })
            .collect()
    }

    /// How far each pile's top is from its target (m), for showing.
    pub fn deviations(&self, i: usize, tops: &[Option<Vec<f64>>]) -> Vec<Option<f64>> {
        let Some(own) = tops[i].as_deref() else {
            return vec![None; self.north[i].len()];
        };
        self.targets(i, tops)
            .iter()
            .zip(own)
            .map(|(t, o)| t.map(|t| o - t))
            .collect()
    }
}

/// Re-optimise trackers pulled towards their neighbours, in rounds.
///
/// `tops` holds the current tops of every tracker (`None` for one without):
/// the fixed ones (e.g. blocks issued to the installers) and the starting
/// point of the others; it is updated as rows move. `work` are the trackers to
/// solve first; afterwards a round solves the movable trackers next to one
/// that moved by more than [`SETTLED`]. Returns the last result of every
/// tracker solved, by index.
#[allow(clippy::too_many_arguments)]
pub fn align_all(
    trackers: &[Tracker],
    rows: &Rows,
    tops: &mut [Option<Vec<f64>>],
    movable: &[bool],
    work: &[usize],
    params: &Params,
    solver: &dyn QpSolver,
    threads: usize,
    rounds: usize,
) -> Vec<(usize, TrackerResult)> {
    let mut out: std::collections::BTreeMap<usize, TrackerResult> = Default::default();
    let mut work: Vec<usize> = work.to_vec();
    for _ in 0..rounds {
        work.retain(|&i| movable[i]);
        if work.is_empty() {
            break;
        }
        let jobs: Vec<(usize, Vec<Option<f64>>)> =
            work.iter().map(|&i| (i, rows.targets(i, tops))).collect();
        let results = par_map(&jobs, threads, |(i, t)| {
            optimize_tracker_aligned(&trackers[*i], params, solver, t)
        });
        let mut moved = BTreeSet::new();
        for ((i, _), res) in jobs.iter().zip(results) {
            let new: Vec<f64> = res.piles.iter().map(|p| p.top).collect();
            let shift = tops[*i].as_ref().map_or(f64::INFINITY, |old| {
                old.iter()
                    .zip(&new)
                    .fold(0.0_f64, |m, (a, b)| m.max((a - b).abs()))
            });
            if shift > SETTLED {
                moved.insert(*i);
            }
            tops[*i] = Some(new);
            out.insert(*i, res);
        }
        let mut next = BTreeSet::new();
        for &i in &moved {
            next.extend(rows.neighbours(i));
            next.insert(i);
        }
        work = next.into_iter().collect();
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Pile;
    use crate::optimize::optimize_tracker;

    /// Rows `pitch` apart, each with piles every 8 m, ground from `ground(row, pile)`.
    fn field(rows: usize, piles: usize, ground: impl Fn(usize, usize) -> f64) -> Vec<Tracker> {
        (0..rows)
            .map(|r| Tracker {
                id: format!("T{r:02}"),
                piles: (0..piles)
                    .map(|k| Pile {
                        name: format!("P{r:02}.{k:02}"),
                        tracker: format!("T{r:02}"),
                        block: "1".into(),
                        section: "1".into(),
                        pile_type: String::new(),
                        easting: 1000.0 + 5.4 * r as f64,
                        northing: 5000.0 + 8.0 * k as f64,
                        z_lidar: ground(r, k),
                        min_top: None,
                    })
                    .collect(),
            })
            .collect()
    }

    fn positions(t: &[Tracker]) -> Vec<Vec<(f64, f64)>> {
        t.iter()
            .map(|t| t.piles.iter().map(|p| (p.easting, p.northing)).collect())
            .collect()
    }

    #[test]
    fn finds_the_row_pitch_and_neighbours() {
        let t = field(4, 6, |_, _| 100.0);
        let rows = Rows::new(&positions(&t));
        assert!((rows.pitch.unwrap() - 5.4).abs() < 1e-9);
        assert_eq!(rows.neighbours(0), vec![1]);
        assert_eq!(rows.neighbours(2), vec![1, 3]);
    }

    #[test]
    fn a_lower_row_is_lifted_towards_its_neighbours() {
        // A steady cross-slope with row 2 sitting 12 cm low: a kink.
        let ground = |r: usize, k: usize| 100.0 + 0.1 * r as f64 + 0.01 * k as f64 - if r == 2 { 0.12 } else { 0.0 };
        let t = field(5, 8, ground);
        let params = Params {
            align_weight: 10.0,
            ..Params::default()
        };
        let solver = crate::optimize::default_solver();
        let mut tops: Vec<Option<Vec<f64>>> = t
            .iter()
            .map(|t| Some(optimize_tracker(t, &params, &solver).piles.iter().map(|p| p.top).collect()))
            .collect();
        let rows = Rows::new(&positions(&t));
        let kink = |tops: &[Option<Vec<f64>>]| {
            rows.deviations(2, tops).iter().flatten().fold(0.0_f64, |m, d| m.max(d.abs()))
        };
        assert!(kink(&tops) > 0.11);
        let movable = vec![true; t.len()];
        let all: Vec<usize> = (0..t.len()).collect();
        let res = align_all(&t, &rows, &mut tops, &movable, &all, &params, &solver, 1, 4);
        assert!(kink(&tops) < 0.03, "kink left {}", kink(&tops));
        for (i, r) in &res {
            assert_ne!(r.status, crate::optimize::TrackerStatus::NonCompliant, "row {i}");
            for p in &r.piles {
                assert!(p.final_reveal >= params.reveal_min() - 1e-4 && p.final_reveal <= params.reveal_max() + 1e-4);
                assert!(p.cut_fill.abs() < 1e-3, "cut/fill {}", p.cut_fill);
            }
        }
    }

    #[test]
    fn fixed_rows_do_not_move() {
        let ground = |r: usize, k: usize| 100.0 + 0.05 * k as f64 - if r == 1 { 0.1 } else { 0.0 };
        let t = field(3, 6, ground);
        let params = Params {
            align_weight: 10.0,
            ..Params::default()
        };
        let solver = crate::optimize::default_solver();
        let mut tops: Vec<Option<Vec<f64>>> = t
            .iter()
            .map(|t| Some(optimize_tracker(t, &params, &solver).piles.iter().map(|p| p.top).collect()))
            .collect();
        let before = tops.clone();
        let rows = Rows::new(&positions(&t));
        let movable = vec![false, true, false];
        let res = align_all(&t, &rows, &mut tops, &movable, &[0, 1, 2], &params, &solver, 1, 4);
        assert_eq!(res.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![1]);
        assert_eq!(tops[0], before[0]);
        assert_eq!(tops[2], before[2]);
        assert_ne!(tops[1], before[1]);
    }
}
