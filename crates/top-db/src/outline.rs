//! Block outlines: one line around all the piles of a block, made only of
//! segments along the trackers and across them (north-south and east-west on
//! a site whose trackers run north-south).
//!
//! The trackers of a block are grouped into columns (trackers in line, one
//! after the other). Each column becomes a strip from `MARGIN` before its
//! first pile to `MARGIN` after its last one; neighbouring strips meet half
//! way between the columns, and the outer ones reach half a row spacing
//! outside. Ends of neighbouring columns that differ by less than `SQUARE` are
//! evened out (always outwards, so no pile is left out), so the line only
//! steps where the trackers really do. A road through a block is crossed: one
//! line per block.

use crate::store::{State, natural_key};
use serde::Serialize;
use std::collections::BTreeMap;

/// The line passes this far (m) beyond the first and last pile of a column.
const MARGIN: f64 = 2.0;
/// Column ends closer than this (m) are evened out into one straight line.
const SQUARE: f64 = 3.0;
/// Trackers whose across-track positions differ by less than this (m) are in
/// the same column.
const SAME_COLUMN: f64 = 1.5;

#[derive(Clone, Debug, Serialize)]
pub struct Outline {
    pub block: String,
    pub piles: usize,
    /// Closed rings (first point repeated at the end), Easting / Northing.
    pub rings: Vec<Vec<(f64, f64)>>,
}

/// Outline of every block of the master list (design positions).
pub fn block_outlines(state: &State) -> Vec<Outline> {
    let mut by_block: BTreeMap<&str, Vec<Vec<(f64, f64)>>> = BTreeMap::new();
    for t in &state.trackers {
        let pts = state.piles[t.first_pile..t.first_pile + t.n_piles]
            .iter()
            .map(|p| (p.easting, p.northing))
            .collect();
        by_block.entry(t.block.as_str()).or_default().push(pts);
    }
    let mut out: Vec<Outline> = by_block
        .into_iter()
        .map(|(block, trackers)| Outline {
            block: block.to_string(),
            piles: trackers.iter().map(Vec::len).sum(),
            rings: outline(&trackers).into_iter().collect(),
        })
        .collect();
    out.sort_by_key(|o| natural_key(&o.block));
    out
}

/// The line around a block's trackers (each a list of pile positions).
pub fn outline(trackers: &[Vec<(f64, f64)>]) -> Option<Vec<(f64, f64)>> {
    let first = *trackers.iter().find(|t| !t.is_empty())?.first()?;
    // Direction of the trackers: the median of their first-to-last bearings.
    let mut angles: Vec<f64> = trackers
        .iter()
        .filter(|t| t.len() > 1)
        .map(|t| {
            let (a, b) = (t[0], t[t.len() - 1]);
            let mut ang = (b.0 - a.0).atan2(b.1 - a.1); // from north, clockwise
            if ang < -std::f64::consts::FRAC_PI_2 {
                ang += std::f64::consts::PI;
            } else if ang > std::f64::consts::FRAC_PI_2 {
                ang -= std::f64::consts::PI;
            }
            ang
        })
        .collect();
    angles.sort_by(f64::total_cmp);
    let theta = angles.get(angles.len() / 2).copied().unwrap_or(0.0);
    let along = (theta.sin(), theta.cos()); // v axis: along the trackers
    let across = (theta.cos(), -theta.sin()); // u axis: across them
    let to_uv = |(x, y): (f64, f64)| {
        let (dx, dy) = (x - first.0, y - first.1);
        (dx * across.0 + dy * across.1, dx * along.0 + dy * along.1)
    };
    let to_xy = |(u, v): (f64, f64)| {
        (
            first.0 + u * across.0 + v * along.0,
            first.1 + u * across.1 + v * along.1,
        )
    };

    // Trackers: across-track position, and their extent along the track.
    let mut tracks: Vec<(f64, f64, f64)> = trackers
        .iter()
        .filter(|t| !t.is_empty())
        .map(|t| {
            let uv: Vec<(f64, f64)> = t.iter().map(|&p| to_uv(p)).collect();
            let u = uv.iter().map(|p| p.0).sum::<f64>() / uv.len() as f64;
            let lo = uv.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
            let hi = uv.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
            (u, lo, hi)
        })
        .collect();
    tracks.sort_by(|a, b| a.0.total_cmp(&b.0));

    // Columns: (u, low end, high end).
    let mut cols: Vec<(f64, f64, f64)> = Vec::new();
    let mut members = 0.0;
    for (u, lo, hi) in tracks {
        match cols.last_mut() {
            Some(c) if u - c.0 < SAME_COLUMN => {
                c.0 = (c.0 * members + u) / (members + 1.0);
                c.1 = c.1.min(lo);
                c.2 = c.2.max(hi);
                members += 1.0;
            }
            _ => {
                cols.push((u, lo, hi));
                members = 1.0;
            }
        }
    }
    for c in &mut cols {
        c.1 -= MARGIN;
        c.2 += MARGIN;
    }

    // Row spacing: the typical gap between neighbouring columns.
    let mut gaps: Vec<f64> = cols.windows(2).map(|w| w[1].0 - w[0].0).collect();
    gaps.sort_by(f64::total_cmp);
    let pitch = gaps.get(gaps.len() / 2).copied().unwrap_or(2.0 * MARGIN);
    let side = (pitch / 2.0).max(MARGIN);

    // Neighbouring strips must overlap, or the line would pinch to nothing.
    for i in 1..cols.len() {
        let (a, b) = (cols[i - 1], cols[i]);
        if a.2 < b.1 + 1.0 {
            cols[i].1 = cols[i].1.min(a.2 - 1.0);
        }
        if b.2 < a.1 + 1.0 {
            cols[i - 1].1 = cols[i - 1].1.min(b.2 - 1.0);
        }
    }

    // Fill notches one or two columns wide, then even out ends that differ by
    // less than SQUARE; both only move the line outwards.
    fill_notches(&mut cols);
    square_off(&mut cols, |c| c.2, |c, v| c.2 = v, true);
    square_off(&mut cols, |c| c.1, |c, v| c.1 = v, false);

    // Strip edges across the track.
    let n = cols.len();
    let left = |i: usize| {
        if i == 0 {
            cols[0].0 - side
        } else {
            (cols[i - 1].0 + cols[i].0) / 2.0
        }
    };
    let right = |i: usize| {
        if i + 1 == n {
            cols[n - 1].0 + side
        } else {
            (cols[i].0 + cols[i + 1].0) / 2.0
        }
    };
    // Anticlockwise: along the low ends, up the far side, back along the
    // high ends, down the near side.
    let mut ring = vec![(left(0), cols[0].1)];
    for i in 0..n {
        ring.push((right(i), cols[i].1));
        if i + 1 < n {
            ring.push((right(i), cols[i + 1].1));
        }
    }
    for i in (0..n).rev() {
        ring.push((right(i), cols[i].2));
        ring.push((left(i), cols[i].2));
        if i > 0 {
            ring.push((left(i), cols[i - 1].2));
        }
    }
    ring.push(ring[0]);
    let ring = drop_straight(&ring);
    Some(
        ring.into_iter()
            .map(|p| {
                let (x, y) = to_xy(p);
                ((x * 100.0).round() / 100.0, (y * 100.0).round() / 100.0)
            })
            .collect(),
    )
}

/// A column (or two side by side) shorter than the columns on both sides
/// makes a narrow notch in the line: extend it to the shorter neighbour.
fn fill_notches(cols: &mut [(f64, f64, f64)]) {
    let n = cols.len();
    for _ in 0..4 {
        let mut changed = false;
        for width in 1..=2 {
            for i in 1..n.saturating_sub(width) {
                let (a, b) = (cols[i - 1], cols[i + width]);
                let inner = &cols[i..i + width];
                let top = a.2.min(b.2);
                if inner.iter().all(|c| c.2 < top - 1e-9) {
                    cols[i..i + width].iter_mut().for_each(|c| c.2 = top);
                    changed = true;
                }
                let inner = &cols[i..i + width];
                let bottom = a.1.max(b.1);
                if inner.iter().all(|c| c.1 > bottom + 1e-9) {
                    cols[i..i + width].iter_mut().for_each(|c| c.1 = bottom);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Groups of neighbouring columns whose ends are within `SQUARE` of each
/// other get the outermost of their ends.
fn square_off(
    cols: &mut [(f64, f64, f64)],
    get: impl Fn(&(f64, f64, f64)) -> f64,
    set: impl Fn(&mut (f64, f64, f64), f64),
    high: bool,
) {
    let mut start = 0;
    while start < cols.len() {
        let (mut lo, mut hi) = (get(&cols[start]), get(&cols[start]));
        let mut end = start + 1;
        while end < cols.len() {
            let v = get(&cols[end]);
            if hi.max(v) - lo.min(v) > SQUARE {
                break;
            }
            lo = lo.min(v);
            hi = hi.max(v);
            end += 1;
        }
        let value = if high { hi } else { lo };
        for c in &mut cols[start..end] {
            set(c, value);
        }
        start = end;
    }
}

/// The ring without repeated points and without points in the middle of a
/// straight run.
fn drop_straight(ring: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let same = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for &p in &ring[..ring.len() - 1] {
        if pts.last().is_none_or(|&q| !same(p, q)) {
            pts.push(p);
        }
    }
    while pts.len() > 1 && same(pts[0], pts[pts.len() - 1]) {
        pts.pop();
    }
    loop {
        let n = pts.len();
        if n < 4 {
            break;
        }
        let straight = (0..n).find(|&i| {
            let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
            cross.abs() < 1e-6
        });
        match straight {
            Some(i) => {
                pts.remove(i);
            }
            None => break,
        }
    }
    let first = pts[0];
    pts.push(first);
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Is the point inside the ring (even-odd rule)?
    fn inside(ring: &[(f64, f64)], (x, y): (f64, f64)) -> bool {
        let mut c = false;
        for w in ring.windows(2) {
            let ((x1, y1), (x2, y2)) = (w[0], w[1]);
            if (y1 > y) != (y2 > y) && x < (x2 - x1) * (y - y1) / (y2 - y1) + x1 {
                c = !c;
            }
        }
        c
    }

    /// North-south trackers: `cols` columns 5.4 m apart, from `y0` with `n` piles 8 m apart.
    fn trackers(x0: f64, cols: usize, y0: impl Fn(usize) -> f64, n: usize) -> Vec<Vec<(f64, f64)>> {
        (0..cols)
            .map(|i| {
                (0..n)
                    .map(|j| (x0 + 5.4 * i as f64, y0(i) + 8.0 * j as f64))
                    .collect()
            })
            .collect()
    }

    fn axis_aligned(ring: &[(f64, f64)]) -> bool {
        ring.windows(2)
            .all(|w| (w[0].0 - w[1].0).abs() < 1e-6 || (w[0].1 - w[1].1).abs() < 1e-6)
    }

    #[test]
    fn straight_lines_along_and_across_the_trackers() {
        // An L: 10 long columns, then 10 half as long; ends staggered by up
        // to 1.5 m, which must not make steps.
        let wobble = |i: usize| [0.0, 1.2, -0.8, 1.5, 0.3][i % 5];
        let mut t = trackers(305_000.0, 10, |i| 6_428_000.0 + wobble(i), 30);
        t.extend(trackers(305_054.0, 10, |i| 6_428_000.0 + wobble(i), 15));
        let r = outline(&t).unwrap();
        assert_eq!(r.first(), r.last());
        assert!(axis_aligned(&r), "{r:?}");
        assert_eq!(r.len() - 1, 6, "an L has 6 corners: {r:?}");
        assert!(t.iter().flatten().all(|&p| inside(&r, p)));
        assert!(
            !inside(&r, (305_080.0, 6_428_200.0)),
            "the notch stays outside"
        );
        // 2 m beyond the first and last piles.
        let low = r.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        assert!((low - (6_428_000.0 - 0.8 - MARGIN)).abs() < 0.011, "{low}");
    }

    #[test]
    fn one_line_across_a_road_and_none_for_nothing() {
        let mut t = trackers(1000.0, 6, |_| 1000.0, 10);
        t.extend(trackers(1000.0, 6, |_| 1107.0, 10)); // 35 m road between
        let r = outline(&t).unwrap();
        assert!(axis_aligned(&r));
        assert_eq!(r.len() - 1, 4, "a rectangle: {r:?}");
        assert!(t.iter().flatten().all(|&p| inside(&r, p)));
        assert!(outline(&[]).is_none());
        let single = outline(&[vec![(5.0, 5.0)]]).unwrap();
        assert!(inside(&single, (5.0, 5.0)));
    }

    #[test]
    fn rotated_trackers_get_lines_along_and_across_them() {
        let a = 20f64.to_radians();
        let rot = |(x, y): (f64, f64)| (x * a.cos() + y * a.sin(), -x * a.sin() + y * a.cos());
        let t: Vec<Vec<(f64, f64)>> = trackers(0.0, 8, |i| if i < 4 { 0.0 } else { 40.0 }, 12)
            .into_iter()
            .map(|tr| tr.into_iter().map(rot).collect())
            .collect();
        let r = outline(&t).unwrap();
        // Two halves offset along the track: a step at each end, 8 corners.
        assert_eq!(r.len() - 1, 8, "{r:?}");
        assert!(t.iter().flatten().all(|&p| inside(&r, p)));
        // Every side is parallel or perpendicular to the trackers.
        for w in r.windows(2) {
            let ang = (w[1].0 - w[0].0).atan2(w[1].1 - w[0].1) - 20f64.to_radians();
            let k = ang / std::f64::consts::FRAC_PI_2;
            assert!((k - k.round()).abs() < 1e-3, "{ang}");
        }
    }
}
