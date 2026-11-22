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
/// A straight line may run at most this far (m) outside the stepped ends it
/// replaces.
const STRAIGHTEN: f64 = 5.0;
/// A straight line keeps at least this far (m) from another block's piles.
const CLEAR: f64 = 1.0;
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
    let all: Vec<(&str, f64, f64)> = state
        .piles
        .iter()
        .map(|p| (p.block.as_str(), p.easting, p.northing))
        .collect();
    let mut out: Vec<Outline> = by_block
        .into_iter()
        .map(|(block, trackers)| {
            // Piles of the other blocks around this one: the line stops half
            // way to them.
            let pts = trackers.iter().flatten();
            let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
            for &(x, y) in pts {
                (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
            }
            const NEAR: f64 = 30.0;
            let others: Vec<(f64, f64)> = all
                .iter()
                .filter(|(b, x, y)| {
                    *b != block && *x > x0 - NEAR && *x < x1 + NEAR && *y > y0 - NEAR && *y < y1 + NEAR
                })
                .map(|&(_, x, y)| (x, y))
                .collect();
            Outline {
                block: block.to_string(),
                piles: trackers.iter().map(Vec::len).sum(),
                rings: outline_among(&trackers, &others),
            }
        })
        .collect();
    out.sort_by_key(|o| natural_key(&o.block));
    out
}

/// The lines around a block's trackers (each a list of pile positions): one,
/// or one per part when the block is in parts side by side, with an empty
/// stretch (often another block) between them.
pub fn outline(trackers: &[Vec<(f64, f64)>]) -> Vec<Vec<(f64, f64)>> {
    outline_among(trackers, &[])
}

/// [`outline`], kept clear of `others` (piles of the neighbouring blocks):
/// the line stops half way to them, at the ends of the rows and beside the
/// outer rows.
pub fn outline_among(trackers: &[Vec<(f64, f64)>], others: &[(f64, f64)]) -> Vec<Vec<(f64, f64)>> {
    let Some(first) = trackers.iter().find_map(|t| t.first().copied()) else {
        return Vec::new();
    };
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
    // The piles' own extent, before the margin.
    let piles_extent: Vec<(f64, f64)> = cols.iter().map(|c| (c.1, c.2)).collect();
    for c in &mut cols {
        c.1 -= MARGIN;
        c.2 += MARGIN;
    }
    let others_uv: Vec<(f64, f64)> = others.iter().map(|&p| to_uv(p)).collect();

    // Row spacing: the typical distance from a column to the nearest column
    // beside it (overlapping along the track). Columns of two groups of rows
    // set between each other (shifted across the track) do not count.
    let overlap = |a: &(f64, f64, f64), b: &(f64, f64, f64)| a.2.min(b.2) - a.1.max(b.1);
    let mut gaps: Vec<f64> = Vec::new();
    for (i, a) in cols.iter().enumerate() {
        if let Some(g) = cols[i + 1..]
            .iter()
            .filter(|b| overlap(a, b) > 0.0)
            .map(|b| b.0 - a.0)
            .find(|g| *g > SAME_COLUMN)
        {
            gaps.push(g);
        }
    }
    gaps.sort_by(f64::total_cmp);
    let pitch = gaps.get(gaps.len() / 2).copied().unwrap_or(2.0 * MARGIN);
    let side = (pitch / 2.0).max(MARGIN);

    // Parts: columns joined when they run beside each other (overlapping
    // along the track) at most one empty row apart. The line never bridges
    // anything else: the gap may hold another block.
    let n = cols.len();
    let mut part_of: Vec<usize> = (0..n).collect();
    fn root(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        p[i] = r;
        r
    }
    for i in 0..n {
        for j in i + 1..n {
            if cols[j].0 - cols[i].0 > 1.6 * pitch {
                break;
            }
            if overlap(&cols[i], &cols[j]) > 0.0 {
                let (a, b) = (root(&mut part_of, i), root(&mut part_of, j));
                part_of[a] = b;
            }
        }
    }
    // How far each column's ends may go: half way to another block's pile
    // in line with it.
    let mut floor = vec![f64::NEG_INFINITY; n];
    let mut ceil = vec![f64::INFINITY; n];
    // A straight line only has to keep clear of their piles.
    let mut floor_line = vec![f64::NEG_INFINITY; n];
    let mut ceil_line = vec![f64::INFINITY; n];
    for (i, c) in cols.iter().enumerate() {
        let (lo, hi) = piles_extent[i];
        for &(u, v) in &others_uv {
            if (u - c.0).abs() < pitch / 2.0 {
                if v < lo {
                    floor[i] = floor[i].max((lo + v) / 2.0);
                    floor_line[i] = floor_line[i].max(v + CLEAR);
                } else if v > hi {
                    ceil[i] = ceil[i].min((hi + v) / 2.0);
                    ceil_line[i] = ceil_line[i].min(v - CLEAR);
                }
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for i in 0..n {
        let r = root(&mut part_of, i);
        groups.entry(r).or_default().push(i);
    }
    groups
        .into_values()
        .map(|idx| {
            let mut part: Vec<(f64, f64, f64)> = idx.iter().map(|&i| cols[i]).collect();
            let fl: Vec<f64> = idx.iter().map(|&i| floor[i]).collect();
            let ce: Vec<f64> = idx.iter().map(|&i| ceil[i]).collect();
            let fll: Vec<f64> = idx.iter().map(|&i| floor_line[i]).collect();
            let cel: Vec<f64> = idx.iter().map(|&i| ceil_line[i]).collect();
            // Beside the outer rows: half way to another block's pile alongside.
            let beside = |c: (f64, f64, f64), (lo, hi): (f64, f64), dir: f64| -> f64 {
                others_uv
                    .iter()
                    .filter(|(u, v)| {
                        let d = dir * (u - c.0);
                        d > 0.0 && d < side + 1e-6 && *v >= lo - MARGIN && *v <= hi + MARGIN
                    })
                    .map(|(u, _)| (c.0 + u) / 2.0)
                    .fold(c.0 + dir * side, |a, b| if dir < 0.0 { a.max(b) } else { a.min(b) })
            };
            let (f, l) = (idx[0], idx[idx.len() - 1]);
            let left_edge = beside(cols[f], piles_extent[f], -1.0);
            let right_edge = beside(cols[l], piles_extent[l], 1.0);
            ring_around(&mut part, side, (&fl, &ce), (&fll, &cel), (left_edge, right_edge))
                .into_iter()
                .map(|p| {
                    let (x, y) = to_xy(p);
                    ((x * 100.0).round() / 100.0, (y * 100.0).round() / 100.0)
                })
                .collect()
        })
        .collect()
}

/// The ring (along / across the track) around neighbouring columns
/// (across-track position, low end, high end).
fn ring_around(
    cols: &mut [(f64, f64, f64)],
    side: f64,
    (floor, ceil): (&[f64], &[f64]),
    (floor_line, ceil_line): (&[f64], &[f64]),
    (left_edge, right_edge): (f64, f64),
) -> Vec<(f64, f64)> {
    // Neighbouring strips must overlap by 1 m at least, or the line would
    // pinch to nothing.
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
    fill_notches(cols);
    square_off(cols, |c| c.2, |c, v| c.2 = v, true);
    square_off(cols, |c| c.1, |c, v| c.1 = v, false);
    // Never past half way to another block.
    for (i, c) in cols.iter_mut().enumerate() {
        c.1 = c.1.max(floor[i]);
        c.2 = c.2.min(ceil[i]);
    }

    // Strip edges across the track.
    let n = cols.len();
    let left = |i: usize| {
        if i == 0 {
            left_edge.max(cols[0].0 - side)
        } else {
            (cols[i - 1].0 + cols[i].0) / 2.0
        }
    };
    let right = |i: usize| {
        if i + 1 == n {
            right_edge.min(cols[n - 1].0 + side)
        } else {
            (cols[i].0 + cols[i + 1].0) / 2.0
        }
    };
    // The low ends and the high ends as steps across the columns, each then
    // straightened where it steps steadily (rows staggered along an edge).
    let mut low = vec![(left(0), cols[0].1)];
    let mut high = vec![(left(0), cols[0].2)];
    for i in 0..n {
        low.push((right(i), cols[i].1));
        high.push((right(i), cols[i].2));
        if i + 1 < n {
            low.push((right(i), cols[i + 1].1));
            high.push((right(i), cols[i + 1].2));
        }
    }
    // How far a straight line may go: clear of another block's piles, checked
    // across each column's strip.
    let lim = |line: &[f64]| -> Vec<(f64, f64)> {
        (0..n)
            .flat_map(|i| [(left(i), line[i]), (cols[i].0, line[i]), (right(i), line[i])])
            .collect()
    };
    let low_lim = lim(floor_line);
    let high_lim = lim(ceil_line);
    let low = straighten(&low, -1.0, &low_lim);
    let high = straighten(&high, 1.0, &high_lim);
    // Anticlockwise: along the low ends, up the far side, back along the
    // high ends, down the near side.
    let mut ring = low;
    ring.extend(high.into_iter().rev());
    ring.push(ring[0]);
    drop_straight(&ring)
}

/// Straight lines in place of steps: from each corner, the longest straight
/// line to a later corner that keeps every corner in between on the inside
/// (`out` = 1: the line is above them, -1: below) and no more than
/// `STRAIGHTEN` from any of them, so real notches stay. Points are in order
/// across the columns.
fn straighten(pts: &[(f64, f64)], out: f64, limits: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let valid = |a: usize, b: usize| -> bool {
        let (pa, pb) = (pts[a], pts[b]);
        if pb.0 - pa.0 < 1e-9 {
            return b == a + 1;
        }
        let y = |x: f64| pa.1 + (pb.1 - pa.1) * (x - pa.0) / (pb.0 - pa.0);
        pts[a + 1..b].iter().all(|p| (-1e-6..=STRAIGHTEN).contains(&(out * (y(p.0) - p.1))))
            && limits
                .iter()
                .filter(|(x, _)| *x >= pa.0 && *x <= pb.0)
                .all(|&(x, lim)| out * (lim - y(x)) >= -1e-6)
    };
    let mut kept = vec![pts[0]];
    let mut a = 0;
    while a + 1 < n {
        let b = (a + 1..n).rev().find(|&b| valid(a, b)).unwrap_or(a + 1);
        kept.push(pts[b]);
        a = b;
    }
    kept
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
        let rings = outline(&t);
        assert_eq!(rings.len(), 1);
        let r = &rings[0];
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
        let rings = outline(&t);
        assert_eq!(rings.len(), 1);
        let r = &rings[0];
        assert!(axis_aligned(&r));
        assert_eq!(r.len() - 1, 4, "a rectangle: {r:?}");
        assert!(t.iter().flatten().all(|&p| inside(&r, p)));
        assert!(outline(&[]).is_empty());
        let single = outline(&[vec![(5.0, 5.0)]]).remove(0);
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
        let rings = outline(&t);
        assert_eq!(rings.len(), 1);
        let r = &rings[0];
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

    #[test]
    fn a_block_around_another_one_is_not_bridged_over_it() {
        // Block 9 of the project: a west part, then four columns of block 8,
        // then a southern part of block 9 further east.
        let mut nine = trackers(0.0, 12, |_| 0.0, 17);
        nine.extend(trackers(81.0, 10, |_| -57.0, 5));
        let eight = trackers(64.8, 4, |_| 0.0, 17);
        let rings = outline(&nine);
        assert_eq!(rings.len(), 2, "{rings:?}");
        assert!(
            nine.iter()
                .flatten()
                .all(|&p| rings.iter().any(|r| inside(r, p)))
        );
        assert!(
            eight
                .iter()
                .flatten()
                .all(|&p| rings.iter().all(|r| !inside(r, p))),
            "no pile of the other block inside"
        );
        assert!(rings.iter().all(|r| axis_aligned(r)));
    }
}
