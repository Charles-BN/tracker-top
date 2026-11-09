//! Per-tracker top-of-pile (ToP) optimisation.
//!
//! For each tracker with piles `i = 0..n` at chainage `s_i` and LIDAR ground
//! `z_i`, we choose ToP heights `y_i` and cut/fill amounts `c_i` to
//!
//! ```text
//! minimise   sum (y_i - c_i - z_i - reveal_nominal)^2  +  lambda * sum c_i^2
//! subject to reveal_min <= y_i - z_i - c_i <= reveal_max          (reveal window)
//!            |c_i| <= max_cut_fill
//!            |slope_i - slope_{i-1}| <= max_angle (radians)        (tracker limit)
//! ```
//!
//! where `slope_i = (y_{i+1} - y_i) / (s_{i+1} - s_i)`. This is a convex QP.
//!
//! The angle limit is imposed on the slope *difference*. Since `atan` is
//! 1-Lipschitz, `|atan(a) - atan(b)| <= |a - b|`, so any solution satisfying
//! the linear constraint also satisfies the true angle limit used by the
//! Python script (`atan2` differences). The linearisation is therefore slightly
//! conservative (well under 1% for slopes up to 10%), never optimistic. The
//! true angles are recomputed and checked after every solve.
//!
//! Replacing the old iterative +/-0.1 m grading loop, cut/fill is a variable
//! with a large penalty `lambda`, so it is used only where the reveal window
//! and angle limit cannot otherwise be met.

use crate::model::Tracker;
use crate::params::Params;
use crate::qp::{AdmmSolver, Qp, QpSolver, QpStatus};
use crate::setout::{set_out, span_error};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackerStatus {
    /// Follows the ground with no meaningful cut/fill and respects all limits.
    Compliant,
    /// Respects all limits but needs cut/fill above `Params::grading_threshold`.
    Graded,
    /// The solver could not find a result that respects all limits.
    NonCompliant,
    /// Fewer than 3 piles: nothing to optimise, ToP = ground + nominal reveal.
    Skipped,
    /// Non-finite coordinates, or two piles at the same position.
    InvalidInput,
}

impl TrackerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TrackerStatus::Compliant => "Compliant",
            TrackerStatus::Graded => "Graded",
            TrackerStatus::NonCompliant => "NonCompliant",
            TrackerStatus::Skipped => "Skipped",
            TrackerStatus::InvalidInput => "InvalidInput",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PileResult {
    /// Optimised top-of-pile elevation (m).
    pub top: f64,
    /// Cut (negative) / fill (positive) applied to the ground (m).
    pub cut_fill: f64,
    /// Ground elevation after cut/fill: `z_lidar + cut_fill`.
    pub new_ground: f64,
    /// `top - new_ground`.
    pub final_reveal: f64,
    /// Angle change at this pile between the two adjacent spans (deg); 0 at the ends.
    pub angle_diff_deg: f64,
    /// LIDAR elevation is an outlier within its tracker.
    pub lidar_outlier: bool,
    /// The top sits on the pile's minimum (`Pile::min_top`, e.g. raised above
    /// the flood level) rather than where the terrain alone would put it.
    pub min_top_active: bool,
    /// Set-out position (m): the design position, or with
    /// `Params::fixed_spans` the position keeping the tube spans' design lengths.
    pub easting: f64,
    pub northing: f64,
    /// Move along the tracker from the design position (m), see [`crate::setout`].
    pub shift: f64,
}

#[derive(Clone, Debug)]
pub struct TrackerResult {
    pub status: TrackerStatus,
    /// Aligned with `Tracker::piles`.
    pub piles: Vec<PileResult>,
    /// Largest |angle change| over the tracker (deg).
    pub max_angle_deg: f64,
    /// Largest |cut/fill| over the tracker (m).
    pub max_cut_fill: f64,
    pub qp_iterations: usize,
    /// Largest |shift| from the design positions (m); 0 without `fixed_spans`.
    pub max_shift: f64,
    /// Largest difference between a span's length along the tube and its
    /// design length (m), after set-out: the verification of the set-out.
    pub max_span_error: f64,
    /// Pile kept at its design position, and whether it is the motor pile.
    pub anchor: Option<(usize, bool)>,
}

/// Angle change (deg) at each pile between the incoming and outgoing span of a
/// profile `(s, y)`. Same definition as the Python script: difference of the
/// `atan2` angles of consecutive spans. First and last entries are 0.
pub fn angle_diffs_deg(s: &[f64], y: &[f64]) -> Vec<f64> {
    let n = y.len();
    let mut out = vec![0.0; n];
    for i in 1..n.saturating_sub(1) {
        let a1 = (y[i] - y[i - 1]).atan2(s[i] - s[i - 1]);
        let a2 = (y[i + 1] - y[i]).atan2(s[i + 1] - s[i]);
        out[i] = (a2 - a1).to_degrees();
    }
    out
}

fn outlier_flags(z: &[f64], alpha: f64) -> Vec<bool> {
    let n = z.len();
    if n < 2 {
        return vec![false; n];
    }
    let mean = z.iter().sum::<f64>() / n as f64;
    let var = z.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    let sd = var.sqrt();
    z.iter().map(|v| (v - mean).abs() > alpha * sd).collect()
}

fn passthrough(
    tracker: &Tracker,
    z: &[f64],
    min_top: &[Option<f64>],
    flags: Vec<bool>,
    params: &Params,
    status: TrackerStatus,
) -> TrackerResult {
    let piles = z
        .iter()
        .zip(min_top)
        .zip(flags)
        .zip(&tracker.piles)
        .map(|(((&z, &mt), lidar_outlier), pile)| {
            let step = params.round_step;
            let nominal = round_to(z + params.reveal_nominal, step);
            let lowest = |m: f64| {
                if step > 0.0 {
                    (m / step - 1e-9).ceil() * step
                } else {
                    m
                }
            };
            let top = mt.map_or(nominal, |m| nominal.max(lowest(m)));
            PileResult {
                top,
                cut_fill: 0.0,
                new_ground: z,
                final_reveal: top - z,
                angle_diff_deg: 0.0,
                lidar_outlier,
                min_top_active: mt.is_some_and(|m| m > nominal),
                easting: pile.easting,
                northing: pile.northing,
                shift: 0.0,
            }
        })
        .collect();
    TrackerResult {
        status,
        piles,
        max_angle_deg: 0.0,
        max_cut_fill: 0.0,
        qp_iterations: 0,
        max_shift: 0.0,
        max_span_error: 0.0,
        anchor: None,
    }
}

/// Assemble the QP described in the module docs. Variables are `[y_0..y_n, c_0..c_n]`,
/// with elevations shifted by `z_ref` for numerical conditioning. Piles with a
/// `min_top` get one more row, `y_i >= min_top_i`.
fn build_qp(s: &[f64], z: &[f64], min_top: &[Option<f64>], z_ref: f64, params: &Params) -> Qp {
    let n = s.len();
    let nv = 2 * n;
    let n_angle = n - 2;
    let n_min = min_top.iter().flatten().count();
    let m = 2 * n + n_angle + n_min;

    let lambda = params.cut_fill_penalty;
    let mut p = vec![0.0; nv * nv];
    let mut q = vec![0.0; nv];
    for i in 0..n {
        let t = z[i] - z_ref + params.reveal_nominal;
        p[i * nv + i] = 2.0;
        p[i * nv + (n + i)] = -2.0;
        p[(n + i) * nv + i] = -2.0;
        p[(n + i) * nv + (n + i)] = 2.0 + 2.0 * lambda;
        q[i] = -2.0 * t;
        q[n + i] = 2.0 * t;
    }

    let mut a = vec![0.0; m * nv];
    let mut l = vec![0.0; m];
    let mut u = vec![0.0; m];
    let mut row = 0;

    // Reveal window: y_i - c_i in [z_i + rmin, z_i + rmax].
    for i in 0..n {
        a[row * nv + i] = 1.0;
        a[row * nv + n + i] = -1.0;
        l[row] = z[i] - z_ref + params.reveal_min();
        u[row] = z[i] - z_ref + params.reveal_max();
        row += 1;
    }

    // Cut/fill cap.
    for i in 0..n {
        a[row * nv + n + i] = 1.0;
        l[row] = -params.max_cut_fill;
        u[row] = params.max_cut_fill;
        row += 1;
    }

    // Angle limit on the slope difference at interior piles, with rows scaled
    // by the mean span length so coefficients are O(1).
    let theta = params.max_angle_deg.to_radians();
    let h_mean = (s[n - 1] - s[0]) / (n as f64 - 1.0);
    for i in 1..n - 1 {
        let h0 = s[i] - s[i - 1];
        let h1 = s[i + 1] - s[i];
        a[row * nv + (i - 1)] = h_mean / h0;
        a[row * nv + i] = -h_mean * (1.0 / h0 + 1.0 / h1);
        a[row * nv + (i + 1)] = h_mean / h1;
        l[row] = -theta * h_mean;
        u[row] = theta * h_mean;
        row += 1;
    }

    // Lowest allowed top of pile (flood level + clearance): y_i >= min_top_i.
    for (i, mt) in min_top.iter().enumerate() {
        if let Some(mt) = mt {
            a[row * nv + i] = 1.0;
            l[row] = mt - z_ref;
            u[row] = f64::INFINITY;
            row += 1;
        }
    }
    debug_assert_eq!(row, m);

    Qp {
        n: nv,
        m,
        p,
        q,
        a,
        l,
        u,
    }
}

/// A top within this distance (m) of its minimum counts as sitting on it.
const MIN_TOP_ACTIVE_TOL: f64 = 1e-3;

/// Optimise one tracker with the given solver.
///
/// With `params.fixed_spans`, the piles are then set out so that every
/// torque-tube span keeps its design length (see [`crate::setout`]), the
/// tracker is optimised again on the set-out spacing until the positions move
/// by less than 0.1 mm, and the true angles are checked at the final positions.
pub fn optimize_tracker(
    tracker: &Tracker,
    params: &Params,
    solver: &dyn QpSolver,
) -> TrackerResult {
    let mut res = optimize_at(tracker, params, solver);
    if !params.fixed_spans || tracker.piles.len() < 2 || res.status == TrackerStatus::InvalidInput {
        return res;
    }
    let tops = |r: &TrackerResult| r.piles.iter().map(|p| p.top).collect::<Vec<f64>>();
    // Positions are rounded (when rounding is on) before the tracker is
    // optimised at them, so that the tops are checked at exactly the
    // positions that will be set out.
    let mut so = set_out(tracker, &tops(&res));
    round_set_out(&mut so, params.round_step);
    let n = tracker.piles.len();
    if n >= 3 {
        const ROUNDS: usize = 6;
        for round in 0..ROUNDS {
            let mut moved = tracker.clone();
            for (i, p) in moved.piles.iter_mut().enumerate() {
                p.easting = so.easting[i];
                p.northing = so.northing[i];
            }
            res = optimize_at(&moved, params, solver);
            let mut next = set_out(tracker, &tops(&res));
            round_set_out(&mut next, params.round_step);
            let change = next
                .shift
                .iter()
                .zip(&so.shift)
                .fold(0.0_f64, |m, (a, b)| m.max((a - b).abs()));
            if change < 1e-4 {
                // `res` was computed at `so`: keep the two together.
                if params.round_step == 0.0 {
                    so = next;
                }
                break;
            }
            if round + 1 == ROUNDS && params.round_step > 0.0 {
                break; // keep the positions `res` was computed at
            }
            so = next;
        }
        // Verification at the final positions: true angles, span lengths.
        let moved = Tracker {
            id: tracker.id.clone(),
            piles: tracker
                .piles
                .iter()
                .enumerate()
                .map(|(i, p)| crate::model::Pile {
                    easting: so.easting[i],
                    northing: so.northing[i],
                    ..p.clone()
                })
                .collect(),
        };
        let angles = angle_diffs_deg(&moved.chainage(), &tops(&res));
        for (p, a) in res.piles.iter_mut().zip(&angles) {
            p.angle_diff_deg = *a;
        }
        res.max_angle_deg = angles.iter().fold(0.0_f64, |m, a| m.max(a.abs()));
        if res.max_angle_deg > params.max_angle_deg + params.angle_tolerance_deg
            && matches!(res.status, TrackerStatus::Compliant | TrackerStatus::Graded)
        {
            res.status = TrackerStatus::NonCompliant;
        }
    }
    round_set_out(&mut so, params.round_step);
    for (i, p) in res.piles.iter_mut().enumerate() {
        p.easting = so.easting[i];
        p.northing = so.northing[i];
        p.shift = so.shift[i];
    }
    res.max_shift = so.shift.iter().fold(0.0_f64, |m, d| m.max(d.abs()));
    res.max_span_error = span_error(tracker, &so, &tops(&res));
    res.anchor = Some((so.anchor, so.anchor_is_motor));
    res
}

/// Optimise one tracker with its piles where they are.
fn optimize_at(tracker: &Tracker, params: &Params, solver: &dyn QpSolver) -> TrackerResult {
    let n = tracker.piles.len();
    let z: Vec<f64> = tracker.piles.iter().map(|p| p.z_lidar).collect();
    let min_top: Vec<Option<f64>> = tracker
        .piles
        .iter()
        .map(|p| p.min_top.filter(|m| m.is_finite()))
        .collect();
    let flags = outlier_flags(&z, params.lidar_sigma_alpha);

    if n < 3 {
        return passthrough(tracker, &z, &min_top, flags, params, TrackerStatus::Skipped);
    }

    let s = tracker.chainage();
    let finite = z.iter().chain(&s).all(|v| v.is_finite());
    let distinct = s.windows(2).all(|w| w[1] - w[0] > 1e-6);
    if !finite || !distinct {
        return passthrough(
            tracker,
            &z,
            &min_top,
            flags,
            params,
            TrackerStatus::InvalidInput,
        );
    }

    let z_ref = z.iter().sum::<f64>() / n as f64;
    let solve = |p: &Params| {
        let sol = solver.solve(&build_qp(&s, &z, &min_top, z_ref, p));
        let top: Vec<f64> = (0..n).map(|i| sol.x[i] + z_ref).collect();
        let cf: Vec<f64> = (0..n).map(|i| sol.x[n + i]).collect();
        (top, cf, sol)
    };
    let (mut top, mut cf, sol) = solve(params);
    let mut solved = sol.status == QpStatus::Solved;
    let mut iterations = sol.iterations;
    let mut on_min: Vec<bool> = (0..n)
        .map(|i| min_top[i].is_some_and(|m| top[i] - m < MIN_TOP_ACTIVE_TOL))
        .collect();

    // Rounding (e.g. to the centimetre): pick, for each pile, a rounded top
    // near the exact one such that every limit still holds on the rounded
    // values. If none exists, solve again with slightly stricter limits so
    // that there is room for the rounding.
    let step = params.round_step;
    if step > 0.0 {
        // A rounding that needs cut/fill the exact solution did not is kept
        // only if no stricter solve gives one without.
        let graded = |c: &[f64]| c.iter().any(|v| v.abs() > params.grading_threshold);
        let exact_graded = graded(&cf);
        let mut fallback: Option<(Vec<f64>, Vec<f64>, Vec<bool>)> = None;
        let mut done = false;
        for level in 0..TIGHTEN.len() {
            let p = tightened(params, level);
            if level > 0 {
                let (t, c, sol) = solve(&p);
                iterations += sol.iterations;
                if sol.status != QpStatus::Solved {
                    continue;
                }
                solved = true;
                on_min = (0..n)
                    .map(|i| min_top[i].is_some_and(|m| t[i] - m < MIN_TOP_ACTIVE_TOL))
                    .collect();
                top = t;
                cf = c;
            }
            if let Some((t, c)) = round_profile(&s, &z, &top, &min_top, params, step) {
                if graded(&c) && !exact_graded {
                    fallback.get_or_insert((t, c, on_min.clone()));
                    continue;
                }
                top = t;
                cf = c;
                done = true;
                break;
            }
        }
        if !done && let Some((t, c, m)) = fallback {
            top = t;
            cf = c;
            on_min = m;
            done = true;
        }
        if !done {
            // Nearest values: the checks below report the tracker NonCompliant.
            for i in 0..n {
                top[i] = round_to(top[i], step);
                cf[i] = cut_fill_for(top[i], z[i], params, step);
            }
        }
    }

    let angles = angle_diffs_deg(&s, &top);
    let max_angle = angles.iter().fold(0.0_f64, |m, a| m.max(a.abs()));
    let max_cf = cf.iter().fold(0.0_f64, |m, c| m.max(c.abs()));

    let piles: Vec<PileResult> = (0..n)
        .map(|i| PileResult {
            top: top[i],
            cut_fill: cf[i],
            new_ground: z[i] + cf[i],
            final_reveal: top[i] - (z[i] + cf[i]),
            angle_diff_deg: angles[i],
            lidar_outlier: flags[i],
            min_top_active: on_min[i],
            easting: tracker.piles[i].easting,
            northing: tracker.piles[i].northing,
            shift: 0.0,
        })
        .collect();

    // Independent check of the true (non-linearised) limits, on the values
    // returned (rounded when rounding is on).
    const REVEAL_TOL: f64 = 1e-4;
    let min_top_ok = piles
        .iter()
        .zip(&min_top)
        .all(|(p, mt)| mt.is_none_or(|m| p.top >= m - REVEAL_TOL));
    let reveal_ok = piles.iter().all(|p| {
        p.final_reveal >= params.reveal_min() - REVEAL_TOL
            && p.final_reveal <= params.reveal_max() + REVEAL_TOL
    });
    let compliant = solved
        && max_angle <= params.max_angle_deg + params.angle_tolerance_deg
        && reveal_ok
        && min_top_ok
        && max_cf <= params.max_cut_fill + REVEAL_TOL;

    let status = if !compliant {
        TrackerStatus::NonCompliant
    } else if max_cf > params.grading_threshold {
        TrackerStatus::Graded
    } else {
        TrackerStatus::Compliant
    };

    TrackerResult {
        status,
        piles,
        max_angle_deg: max_angle,
        max_cut_fill: max_cf,
        qp_iterations: iterations,
        max_shift: 0.0,
        max_span_error: 0.0,
        anchor: None,
    }
}

/// Set-out positions and shifts rounded to `step` (nothing when 0).
fn round_set_out(so: &mut crate::setout::SetOut, step: f64) {
    for v in so
        .easting
        .iter_mut()
        .chain(so.northing.iter_mut())
        .chain(so.shift.iter_mut())
    {
        *v = round_to(*v, step);
    }
}

/// `v` rounded to the nearest multiple of `step`.
pub fn round_to(v: f64, step: f64) -> f64 {
    if step > 0.0 {
        (v / step).round() * step
    } else {
        v
    }
}

/// Angle-limit reductions (degrees) tried in turn when no rounding fits.
const TIGHTEN: [f64; 4] = [0.0, 0.02, 0.06, 0.15];

/// The limits with some room for rounding: level 0 is `params` itself; each
/// level narrows the angle limit and the reveal window a little more.
fn tightened(params: &Params, level: usize) -> Params {
    let k = level as f64;
    let step = params.round_step;
    let mut p = params.clone();
    p.max_angle_deg = (params.max_angle_deg - TIGHTEN[level]).max(params.max_angle_deg * 0.5);
    let room = (params.delta_max - params.delta_min) / 4.0;
    p.delta_min = params.delta_min + (0.5 * step * k).min(room);
    p.delta_max = params.delta_max - (0.5 * step * k).min(room);
    p
}

/// Cut (negative) or fill (positive), a multiple of `step`, that brings the
/// reveal of a pile topped at `top` into the window: 0 when it already is.
fn cut_fill_for(top: f64, z: f64, params: &Params, step: f64) -> f64 {
    const EPS: f64 = 1e-9;
    let reveal = top - z;
    if reveal > params.reveal_max() + EPS {
        ((reveal - params.reveal_max()) / step - EPS).ceil() * step
    } else if reveal < params.reveal_min() - EPS {
        ((reveal - params.reveal_min()) / step + EPS).floor() * step
    } else {
        0.0
    }
}

/// Rounded tops (multiples of `step`) close to the exact tops `y` that keep
/// every limit: reveal window (with rounded cut/fill where needed), cut/fill
/// cap, lowest top, and the true angle change at every interior pile.
///
/// Each pile may take one of the six multiples of `step` around its exact
/// top. The angle at a pile depends on its two neighbours, so a dynamic
/// programme over pairs of consecutive choices finds the combination closest
/// to the exact tops (squared differences, cut/fill weighted like in the
/// optimisation) in a few hundred steps. `None` when no combination fits.
fn round_profile(
    s: &[f64],
    z: &[f64],
    y: &[f64],
    min_top: &[Option<f64>],
    params: &Params,
    step: f64,
) -> Option<(Vec<f64>, Vec<f64>)> {
    let n = y.len();
    // Candidates per pile: (top, cut/fill, cost).
    let cands: Vec<Vec<(f64, f64, f64)>> = (0..n)
        .map(|i| {
            let base = (y[i] / step).floor() as i64;
            (-2..=3)
                .filter_map(|k| {
                    let v = (base + k) as f64 * step;
                    if min_top[i].is_some_and(|m| v < m - 1e-9) {
                        return None;
                    }
                    let c = cut_fill_for(v, z[i], params, step);
                    if c.abs() > params.max_cut_fill + 1e-9 {
                        return None;
                    }
                    let cost = (v - y[i]).powi(2) + params.cut_fill_penalty * c * c;
                    Some((v, c, cost))
                })
                .collect()
        })
        .collect();
    if cands.iter().any(Vec::is_empty) {
        return None;
    }
    let limit = params.max_angle_deg + params.angle_tolerance_deg;
    let angle_ok = |i: usize, a: f64, b: f64, c: f64| {
        let a1 = (b - a).atan2(s[i] - s[i - 1]);
        let a2 = (c - b).atan2(s[i + 1] - s[i]);
        (a2 - a1).to_degrees().abs() <= limit
    };
    // best[a][b]: lowest cost of piles 0..=i with choice a at i-1 and b at i.
    const K: usize = 6;
    let mut best = [[f64::INFINITY; K]; K];
    for (a, ca) in cands[0].iter().enumerate() {
        for (b, cb) in cands[1].iter().enumerate() {
            best[a][b] = ca.2 + cb.2;
        }
    }
    // back[i][b][c]: the choice at pile i-1 on the best path to (b at i, c at i+1).
    let mut back = vec![[[usize::MAX; K]; K]; n];
    for i in 1..n - 1 {
        let mut next = [[f64::INFINITY; K]; K];
        for (a, ca) in cands[i - 1].iter().enumerate() {
            for (b, cb) in cands[i].iter().enumerate() {
                if !best[a][b].is_finite() {
                    continue;
                }
                for (c, cc) in cands[i + 1].iter().enumerate() {
                    if !angle_ok(i, ca.0, cb.0, cc.0) {
                        continue;
                    }
                    let cost = best[a][b] + cc.2;
                    if cost < next[b][c] {
                        next[b][c] = cost;
                        back[i + 1][b][c] = a;
                    }
                }
            }
        }
        best = next;
    }
    let (mut b, mut c, mut cost) = (0, 0, f64::INFINITY);
    for (i, row) in best.iter().enumerate() {
        for (j, v) in row.iter().enumerate() {
            if *v < cost {
                (b, c, cost) = (i, j, *v);
            }
        }
    }
    if !cost.is_finite() {
        return None;
    }
    let mut choice = vec![0; n];
    choice[n - 1] = c;
    choice[n - 2] = b;
    for i in (2..n).rev() {
        choice[i - 2] = back[i][choice[i - 1]][choice[i]];
    }
    Some((
        (0..n).map(|i| cands[i][choice[i]].0).collect(),
        (0..n).map(|i| cands[i][choice[i]].1).collect(),
    ))
}

/// Optimise all trackers on `threads` worker threads. Results are returned in
/// the same order as `trackers`. With one thread no thread is started, so this
/// also runs where threads are unavailable (WebAssembly in a browser).
pub fn optimize_all(
    trackers: &[Tracker],
    params: &Params,
    solver: &dyn QpSolver,
    threads: usize,
) -> Vec<TrackerResult> {
    let threads = threads.max(1).min(trackers.len().max(1));
    if threads == 1 {
        return trackers
            .iter()
            .map(|t| optimize_tracker(t, params, solver))
            .collect();
    }
    let next = AtomicUsize::new(0);

    let mut slots: Vec<Option<TrackerResult>> = (0..trackers.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= trackers.len() {
                            break;
                        }
                        local.push((i, optimize_tracker(&trackers[i], params, solver)));
                    }
                    local
                })
            })
            .collect();
        for h in handles {
            for (i, r) in h.join().expect("worker thread panicked") {
                slots[i] = Some(r);
            }
        }
    });
    slots
        .into_iter()
        .map(|r| r.expect("every tracker is processed"))
        .collect()
}

/// Convenience: default ADMM solver.
pub fn default_solver() -> AdmmSolver {
    AdmmSolver::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Pile;

    fn tracker(id: &str, z: &[f64], spacing: f64) -> Tracker {
        let piles = z
            .iter()
            .enumerate()
            .map(|(i, &z)| Pile {
                name: format!("{id}.{:02}", i + 1),
                tracker: id.into(),
                block: "1".into(),
                section: "1".into(),
                pile_type: String::new(),
                easting: 1000.0,
                northing: 5000.0 + spacing * i as f64,
                z_lidar: z,
                min_top: None,
            })
            .collect();
        Tracker {
            id: id.into(),
            piles,
        }
    }

    fn check(tr: &Tracker, r: &TrackerResult, params: &Params) {
        assert_ne!(
            r.status,
            TrackerStatus::NonCompliant,
            "{}: {:?}",
            tr.id,
            r.max_angle_deg
        );
        assert!(
            r.max_angle_deg <= params.max_angle_deg + 1e-6,
            "angle {}",
            r.max_angle_deg
        );
        for p in &r.piles {
            assert!(
                p.final_reveal >= params.reveal_min() - 1e-4,
                "reveal {}",
                p.final_reveal
            );
            assert!(
                p.final_reveal <= params.reveal_max() + 1e-4,
                "reveal {}",
                p.final_reveal
            );
            assert!(p.cut_fill.abs() <= params.max_cut_fill + 1e-4);
        }
    }

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
        }
    }

    #[test]
    fn flat_ground_gives_nominal_reveal() {
        let params = Params::default();
        let tr = tracker("T", &[100.0; 15], 8.0);
        let r = optimize_tracker(&tr, &params, &AdmmSolver::default());
        assert_eq!(r.status, TrackerStatus::Compliant);
        for p in &r.piles {
            assert!((p.top - 101.45).abs() < 1e-5, "{}", p.top);
            assert!(p.cut_fill.abs() < 1e-5);
        }
    }

    #[test]
    fn uniform_slope_is_followed_exactly() {
        let params = Params::default();
        let z: Vec<f64> = (0..15).map(|i| 100.0 + 0.02 * 8.0 * i as f64).collect();
        let tr = tracker("T", &z, 8.0);
        let r = optimize_tracker(&tr, &params, &AdmmSolver::default());
        assert_eq!(r.status, TrackerStatus::Compliant);
        for (p, z) in r.piles.iter().zip(&z) {
            assert!((p.top - (z + 1.45)).abs() < 1e-5);
        }
        assert!(r.max_angle_deg < 1e-4);
    }

    #[test]
    fn small_bump_is_smoothed_within_the_reveal_window() {
        let params = Params::default();
        let mut z = vec![100.0; 15];
        z[7] = 99.85; // 15 cm dip: needs smoothing but no earthworks
        let tr = tracker("T", &z, 8.0);
        let r = optimize_tracker(&tr, &params, &AdmmSolver::default());
        check(&tr, &r, &params);
        assert_eq!(
            r.status,
            TrackerStatus::Compliant,
            "max cf {}",
            r.max_cut_fill
        );
        // The pile above the dip must have a larger reveal than its neighbours.
        assert!(r.piles[7].final_reveal > r.piles[3].final_reveal);
    }

    #[test]
    fn deep_dip_forces_grading() {
        let params = Params::default();
        let mut z = vec![100.0; 15];
        z[7] = 99.3; // 70 cm dip cannot be followed by the tracker
        let tr = tracker("T", &z, 8.0);
        let r = optimize_tracker(&tr, &params, &AdmmSolver::default());
        check(&tr, &r, &params);
        assert_eq!(r.status, TrackerStatus::Graded);
        assert!(r.piles[7].cut_fill > 0.05, "expected fill at the dip");
    }

    #[test]
    fn uneven_spacing_is_handled() {
        let params = Params::default();
        let mut tr = tracker("T", &[100.0, 100.1, 100.0, 100.2, 100.1, 100.0], 8.0);
        tr.piles[3].northing += 2.0;
        tr.piles[4].northing += 2.5;
        tr.piles[5].northing += 3.0;
        let r = optimize_tracker(&tr, &params, &AdmmSolver::default());
        check(&tr, &r, &params);
    }

    #[test]
    fn fewer_than_three_piles_are_skipped() {
        let tr = tracker("T", &[100.0, 100.2], 8.0);
        let r = optimize_tracker(&tr, &Params::default(), &AdmmSolver::default());
        assert_eq!(r.status, TrackerStatus::Skipped);
        assert!((r.piles[1].top - 101.65).abs() < 1e-12);
    }

    #[test]
    fn duplicate_positions_are_rejected() {
        let mut tr = tracker("T", &[100.0, 100.0, 100.0, 100.0], 8.0);
        tr.piles[2].northing = tr.piles[1].northing;
        let r = optimize_tracker(&tr, &Params::default(), &AdmmSolver::default());
        assert_eq!(r.status, TrackerStatus::InvalidInput);
    }

    #[test]
    fn lidar_outliers_are_flagged() {
        let mut z = vec![100.0; 15];
        z[4] = 103.0;
        let tr = tracker("T", &z, 8.0);
        let r = optimize_tracker(&tr, &Params::default(), &AdmmSolver::default());
        assert!(r.piles[4].lidar_outlier);
        assert!(!r.piles[0].lidar_outlier);
    }

    #[test]
    fn random_terrain_always_respects_the_limits() {
        let params = Params::default();
        let mut rng = Lcg(42);
        let mut trackers = Vec::new();
        for k in 0..300 {
            let n = 5 + (rng.next() * 11.0) as usize; // 5..=15 piles
            let mut z = Vec::new();
            let mut level = 50.0 + 10.0 * rng.next();
            let mut slope = 0.08 * (rng.next() - 0.5);
            for _ in 0..n {
                slope += 0.03 * (rng.next() - 0.5);
                level += slope * 8.0;
                let noise = 0.12 * (rng.next() - 0.5);
                let spike = if rng.next() < 0.03 {
                    0.6 * (rng.next() - 0.5)
                } else {
                    0.0
                };
                z.push(level + noise + spike);
            }
            trackers.push(tracker(&format!("R{k:03}"), &z, 8.0));
        }
        let solver = AdmmSolver::default();
        let results = optimize_all(&trackers, &params, &solver, 4);
        assert_eq!(results.len(), trackers.len());
        for (tr, r) in trackers.iter().zip(&results) {
            check(tr, r, &params);
        }
        // Parallel run must match a serial run exactly.
        let serial: Vec<_> = trackers
            .iter()
            .map(|t| optimize_tracker(t, &params, &solver))
            .collect();
        for (a, b) in results.iter().zip(&serial) {
            assert_eq!(a.status, b.status);
            for (pa, pb) in a.piles.iter().zip(&b.piles) {
                assert_eq!(pa.top, pb.top);
            }
        }
    }

    fn with_min_top(mut tr: Tracker, min_top: &[Option<f64>]) -> Tracker {
        for (p, m) in tr.piles.iter_mut().zip(min_top) {
            p.min_top = *m;
        }
        tr
    }

    #[test]
    fn flood_level_raises_the_bottom_of_the_window() {
        // Flat ground, flood level needs the middle piles 1.50 m above ground
        // (inside the 1.20-1.60 window): they rise, no earthworks.
        let params = Params::default();
        let mut mt = vec![None; 15];
        for m in mt.iter_mut().take(9).skip(6) {
            *m = Some(101.50);
        }
        let tr = with_min_top(tracker("T", &[100.0; 15], 8.0), &mt);
        let r = optimize_tracker(&tr, &params, &AdmmSolver::default());
        check(&tr, &r, &params);
        assert_eq!(r.status, TrackerStatus::Compliant);
        for (i, p) in r.piles.iter().enumerate() {
            if (6..9).contains(&i) {
                assert!(p.top >= 101.50 - 1e-4, "pile {i}: {}", p.top);
                assert!(p.min_top_active, "pile {i}");
            }
        }
        assert!(r.piles.iter().all(|p| p.cut_fill.abs() < 1e-4));
        assert!(!r.piles[0].min_top_active);
    }

    #[test]
    fn flood_above_the_window_needs_fill() {
        // ToP must be 1.80 m above ground everywhere: more than the 1.60 m
        // maximum reveal, so the ground is raised by about 0.20 m.
        let params = Params::default();
        let tr = with_min_top(tracker("T", &[100.0; 12], 8.0), &[Some(101.80); 12]);
        let r = optimize_tracker(&tr, &params, &AdmmSolver::default());
        check(&tr, &r, &params);
        assert_eq!(r.status, TrackerStatus::Graded);
        for p in &r.piles {
            assert!(p.top >= 101.80 - 1e-4);
            assert!((p.cut_fill - 0.20).abs() < 2e-3, "fill {}", p.cut_fill);
            assert!(p.min_top_active);
        }
    }

    #[test]
    fn unreachable_flood_level_is_non_compliant() {
        // 3 m above ground: needs 1.4 m of fill, above the 1 m cap.
        let params = Params::default();
        let tr = with_min_top(tracker("T", &[100.0; 9], 8.0), &[Some(103.0); 9]);
        let r = optimize_tracker(&tr, &params, &AdmmSolver::default());
        assert_eq!(r.status, TrackerStatus::NonCompliant);
    }

    #[test]
    fn short_trackers_respect_the_flood_level() {
        let params = Params::default();
        let tr = with_min_top(tracker("T", &[100.0, 100.1], 8.0), &[Some(101.55), None]);
        let r = optimize_tracker(&tr, &params, &AdmmSolver::default());
        assert_eq!(r.status, TrackerStatus::Skipped);
        assert!((r.piles[0].top - 101.55).abs() < 1e-9 && r.piles[0].min_top_active);
        assert!((r.piles[1].top - 101.55).abs() < 1e-9 && !r.piles[1].min_top_active);
    }

    #[test]
    fn fixed_spans_set_out_and_verify() {
        // 15-pile tracker on a 6 % slope, motor pile in the middle.
        let params = Params {
            fixed_spans: true,
            ..Params::default()
        };
        let mut tr = tracker(
            "T",
            &(0..15)
                .map(|i| 100.0 + 0.06 * 9.415 * i as f64)
                .collect::<Vec<_>>(),
            9.415,
        );
        tr.piles[7].pile_type = "HMP-20".into();
        let r = optimize_tracker(&tr, &params, &AdmmSolver::default());
        check(&tr, &r, &params);
        assert_eq!(r.anchor, Some((7, true)));
        assert!(r.piles[7].shift.abs() < 1e-12);
        // Each span 16.9 mm shorter in plan: 7 spans from the motor pile to each end.
        let per = 9.415 - (9.415f64.powi(2) - (0.06 * 9.415f64).powi(2)).sqrt();
        assert!(
            (r.piles[0].shift - 7.0 * per).abs() < 1e-4,
            "{}",
            r.piles[0].shift
        );
        assert!(
            (r.piles[14].shift + 7.0 * per).abs() < 1e-4,
            "{}",
            r.piles[14].shift
        );
        assert!((r.max_shift - 7.0 * per).abs() < 1e-4);
        assert!(r.max_span_error < 1e-6, "{}", r.max_span_error);
        assert!((r.piles[14].northing - (5000.0 + 14.0 * 9.415 - 7.0 * per)).abs() < 1e-4);

        // Without the option, positions stay as designed.
        let r = optimize_tracker(&tr, &Params::default(), &AdmmSolver::default());
        assert!(r.piles.iter().all(|p| p.shift == 0.0) && r.anchor.is_none());
    }

    #[test]
    fn set_out_keeps_angles_within_the_limit() {
        // Bumpy ground: angles sit on the limit, and must still be within it
        // at the set-out positions.
        let params = Params {
            fixed_spans: true,
            ..Params::default()
        };
        let mut rng = Lcg(42);
        for k in 0..40 {
            let z: Vec<f64> = (0..15)
                .map(|i| 100.0 + 0.08 * 9.0 * i as f64 + 0.4 * (rng.next() - 0.5))
                .collect();
            let mut tr = tracker(&format!("T{k}"), &z, 9.0);
            tr.piles[7].pile_type = "SMP-20".into();
            let r = optimize_tracker(&tr, &params, &AdmmSolver::default());
            assert_ne!(
                r.status,
                TrackerStatus::NonCompliant,
                "T{k}: {}",
                r.max_angle_deg
            );
            assert!(
                r.max_angle_deg <= params.max_angle_deg + params.angle_tolerance_deg,
                "T{k}: {}",
                r.max_angle_deg
            );
            assert!(r.max_span_error < 1e-6, "T{k}: {}", r.max_span_error);
        }
    }

    fn on_grid(v: f64, step: f64) -> bool {
        ((v / step) - (v / step).round()).abs() < 1e-6
    }

    fn random_trackers(seed: u64, count: usize, spacing: f64) -> Vec<Tracker> {
        let mut rng = Lcg(seed);
        (0..count)
            .map(|k| {
                let n = 5 + (rng.next() * 11.0) as usize;
                let mut z = Vec::new();
                let mut level = 50.0 + 10.0 * rng.next();
                let mut slope = 0.08 * (rng.next() - 0.5);
                for _ in 0..n {
                    slope += 0.03 * (rng.next() - 0.5);
                    level += slope * spacing;
                    let spike = if rng.next() < 0.03 {
                        0.6 * (rng.next() - 0.5)
                    } else {
                        0.0
                    };
                    z.push(level + 0.12 * (rng.next() - 0.5) + spike);
                }
                tracker(&format!("R{k:03}"), &z, spacing)
            })
            .collect()
    }

    #[test]
    fn rounded_to_the_centimetre_and_still_within_every_limit() {
        let exact = Params::default();
        let params = Params {
            round_step: 0.01,
            ..Params::default()
        };
        let solver = AdmmSolver::default();
        let trackers = random_trackers(42, 300, 8.0);
        let rounded = optimize_all(&trackers, &params, &solver, 4);
        let mut moved = 0.0_f64;
        for (tr, r) in trackers.iter().zip(&rounded) {
            check(tr, r, &params);
            let e = optimize_tracker(tr, &exact, &solver);
            for (p, q) in r.piles.iter().zip(&e.piles) {
                assert!(on_grid(p.top, 0.01), "{}: top {}", tr.id, p.top);
                assert!(
                    on_grid(p.cut_fill, 0.01),
                    "{}: cut/fill {}",
                    tr.id,
                    p.cut_fill
                );
                moved = moved.max((p.top - q.top).abs());
            }
        }
        // Close to the exact optimum. Where rounding alone would need a
        // centimetre of earthworks, a slightly stricter solve avoids it by
        // moving some tops a few centimetres within the reveal window.
        assert!(moved < 0.08, "largest change from rounding {moved}");
    }

    #[test]
    fn rounding_respects_flood_minimums_and_short_trackers() {
        let params = Params {
            round_step: 0.01,
            ..Params::default()
        };
        let solver = AdmmSolver::default();
        let mut mt = vec![None; 15];
        for m in mt.iter_mut().take(9).skip(6) {
            *m = Some(101.503);
        }
        let tr = with_min_top(tracker("T", &[100.0; 15], 8.0), &mt);
        let r = optimize_tracker(&tr, &params, &solver);
        check(&tr, &r, &params);
        for i in 6..9 {
            assert!(r.piles[i].top >= 101.503 && on_grid(r.piles[i].top, 0.01));
            assert!(r.piles[i].min_top_active);
        }
        let short = with_min_top(tracker("S", &[100.004, 100.1], 8.0), &[Some(101.551), None]);
        let r = optimize_tracker(&short, &params, &solver);
        assert_eq!(r.status, TrackerStatus::Skipped);
        assert!((r.piles[0].top - 101.56).abs() < 1e-9, "{}", r.piles[0].top);
        assert!((r.piles[1].top - 101.55).abs() < 1e-9, "{}", r.piles[1].top);
    }

    #[test]
    fn rounded_set_out_keeps_spans_to_the_centimetre() {
        let params = Params {
            round_step: 0.01,
            fixed_spans: true,
            ..Params::default()
        };
        let mut rng = Lcg(7);
        for k in 0..30 {
            let z: Vec<f64> = (0..15)
                .map(|i| 100.0 + 0.08 * 9.0 * i as f64 + 0.4 * (rng.next() - 0.5))
                .collect();
            let mut tr = tracker(&format!("T{k}"), &z, 9.0);
            tr.piles[7].pile_type = "SMP-20".into();
            let r = optimize_tracker(&tr, &params, &AdmmSolver::default());
            check(&tr, &r, &params);
            for p in &r.piles {
                assert!(
                    on_grid(p.top, 0.01) && on_grid(p.easting, 0.01) && on_grid(p.northing, 0.01)
                );
            }
            // Rounding each end of a span by up to 5 mm.
            assert!(r.max_span_error <= 0.0101, "T{k}: {}", r.max_span_error);
        }
    }
}
