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
/// Corners looked at ahead when straightening.
const LOOK_AHEAD: usize = 120;
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

    let others_uv: Vec<(f64, f64)> = others.iter().map(|&p| to_uv(p)).collect();

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

    // Columns: trackers in line, with the stretch each one covers.
    let mut cols: Vec<Column> = Vec::new();
    let mut members = 0.0;
    for (u, lo, hi) in tracks {
        match cols.last_mut() {
            Some(c) if u - c.u < SAME_COLUMN => {
                c.u = (c.u * members + u) / (members + 1.0);
                c.pieces.push(Piece::new(lo, hi));
                members += 1.0;
            }
            _ => {
                cols.push(Column {
                    u,
                    pieces: vec![Piece::new(lo, hi)],
                });
                members = 1.0;
            }
        }
    }
    for c in &mut cols {
        c.pieces.sort_by(|a, b| a.lo.total_cmp(&b.lo));
    }

    // Row spacing: the typical distance from a column to the nearest column
    // beside it (overlapping along the track). Columns of two groups of rows
    // set between each other (shifted across the track) do not count.
    let extent = |c: &Column| (c.u, c.lo() - MARGIN, c.hi() + MARGIN);
    let overlap = |a: (f64, f64, f64), b: (f64, f64, f64)| a.2.min(b.2) - a.1.max(b.1);
    let mut gaps: Vec<f64> = Vec::new();
    for (i, a) in cols.iter().enumerate() {
        if let Some(g) = cols[i + 1..]
            .iter()
            .filter(|b| overlap(extent(a), extent(b)) > 0.0)
            .map(|b| b.u - a.u)
            .find(|g| *g > SAME_COLUMN)
        {
            gaps.push(g);
        }
    }
    gaps.sort_by(f64::total_cmp);
    let pitch = gaps.get(gaps.len() / 2).copied().unwrap_or(2.0 * MARGIN);
    let side = (pitch / 2.0).max(MARGIN);

    // The trackers of a column form one stretch, unless another block's pile
    // lies in line between them: the line then goes around it. Each stretch
    // reaches MARGIN beyond its piles, but never past half way to another
    // block's pile in line with it.
    for c in &mut cols {
        let in_line: Vec<f64> = others_uv
            .iter()
            .filter(|(u, _)| (u - c.u).abs() < pitch / 2.0)
            .map(|&(_, v)| v)
            .collect();
        let mut joined: Vec<Piece> = Vec::new();
        for p in c.pieces.drain(..) {
            match joined.last_mut() {
                Some(last)
                    if p.lo <= last.hi || !in_line.iter().any(|&v| v > last.hi && v < p.lo) =>
                {
                    last.hi = last.hi.max(p.hi);
                }
                _ => joined.push(p),
            }
        }
        for p in &mut joined {
            for &v in &in_line {
                if v < p.lo {
                    p.floor = p.floor.max((p.lo + v) / 2.0);
                } else if v > p.hi {
                    p.ceil = p.ceil.min((p.hi + v) / 2.0);
                }
            }
            p.lo = (p.lo - MARGIN).max(p.floor);
            p.hi = (p.hi + MARGIN).min(p.ceil);
        }
        c.pieces = joined;
    }

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
            if cols[j].u - cols[i].u > 1.6 * pitch {
                break;
            }
            if overlap(extent(&cols[i]), extent(&cols[j])) > 0.0 {
                let (a, b) = (root(&mut part_of, i), root(&mut part_of, j));
                part_of[a] = b;
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for i in 0..n {
        let r = root(&mut part_of, i);
        groups.entry(r).or_default().push(i);
    }
    let mut rings = Vec::new();
    for idx in groups.into_values() {
        let mut part: Vec<Column> = idx.iter().map(|&i| cols[i].clone()).collect();
        // Beside the outer rows: half way to another block's pile alongside.
        let beside = |c: &Column, dir: f64| -> f64 {
            others_uv
                .iter()
                .filter(|(u, v)| {
                    let d = dir * (u - c.u);
                    d > 0.0 && d < side + 1e-6 && *v >= c.lo() && *v <= c.hi()
                })
                .map(|(u, _)| (c.u + u) / 2.0)
                .fold(c.u + dir * side, |a, b| if dir < 0.0 { a.max(b) } else { a.min(b) })
        };
        let edges = (beside(&part[0], -1.0), beside(&part[part.len() - 1], 1.0));
        for ring in rings_around(&mut part, side, edges) {
            let ring = straighten_ring(&ring, &others_uv);
            rings.push(
                drop_straight(&ring)
                    .into_iter()
                    .map(|p| {
                        let (x, y) = to_xy(p);
                        ((x * 100.0).round() / 100.0, (y * 100.0).round() / 100.0)
                    })
                    .collect(),
            );
        }
    }
    rings
}

/// A stretch of a column along the track, and how far its ends may go (half
/// way to another block's pile in line with it).
#[derive(Clone, Copy, Debug)]
struct Piece {
    lo: f64,
    hi: f64,
    floor: f64,
    ceil: f64,
}

impl Piece {
    fn new(lo: f64, hi: f64) -> Self {
        Piece {
            lo,
            hi,
            floor: f64::NEG_INFINITY,
            ceil: f64::INFINITY,
        }
    }
}

/// Trackers in line: their across-track position and the stretches they cover.
#[derive(Clone, Debug)]
struct Column {
    u: f64,
    pieces: Vec<Piece>,
}

impl Column {
    fn lo(&self) -> f64 {
        self.pieces.first().map_or(0.0, |p| p.lo)
    }
    fn hi(&self) -> f64 {
        self.pieces.last().map_or(0.0, |p| p.hi)
    }
}

/// The rings (along / across the track) around neighbouring columns: their
/// strips (half way to the next column; beside the outer ones, `side` or up
/// to `edges`) over the stretches they cover, joined.
fn rings_around(cols: &mut [Column], side: f64, (left_edge, right_edge): (f64, f64)) -> Vec<Vec<(f64, f64)>> {
    let n = cols.len();
    // The outer ends of each column, as (across, low end, high end).
    let mut ends: Vec<(f64, f64, f64)> = cols.iter().map(|c| (c.u, c.lo(), c.hi())).collect();
    // Neighbouring strips must overlap by 1 m at least, or the line would
    // pinch to nothing.
    for i in 1..n {
        let (a, b) = (ends[i - 1], ends[i]);
        if a.2 < b.1 + 1.0 {
            ends[i].1 = ends[i].1.min(a.2 - 1.0);
        }
        if b.2 < a.1 + 1.0 {
            ends[i - 1].1 = ends[i - 1].1.min(b.2 - 1.0);
        }
    }
    // Fill notches one or two columns wide, then even out ends that differ by
    // less than SQUARE; both only move the line outwards, never past half way
    // to another block.
    fill_notches(&mut ends);
    square_off(&mut ends, |c| c.2, |c, v| c.2 = v, true);
    square_off(&mut ends, |c| c.1, |c, v| c.1 = v, false);
    for (c, e) in cols.iter_mut().zip(&ends) {
        let first = c.pieces.first_mut().expect("a column has a stretch");
        first.lo = e.1.max(first.floor).min(first.lo);
        let last = c.pieces.last_mut().expect("a column has a stretch");
        last.hi = e.2.min(last.ceil).max(last.hi);
    }

    // Strips across the track.
    let left = |i: usize| {
        if i == 0 {
            left_edge.max(cols[0].u - side)
        } else {
            (cols[i - 1].u + cols[i].u) / 2.0
        }
    };
    let right = |i: usize| {
        if i + 1 == n {
            right_edge.min(cols[n - 1].u + side)
        } else {
            (cols[i].u + cols[i + 1].u) / 2.0
        }
    };
    let rects: Vec<(f64, f64, f64, f64)> = (0..n)
        .flat_map(|i| cols[i].pieces.iter().map(move |p| (left(i), right(i), p.lo, p.hi)))
        .collect();
    union_rings(&rects)
}

/// Outer boundaries (anticlockwise) of the union of rectangles (x0, x1, y0, y1).
fn union_rings(rects: &[(f64, f64, f64, f64)]) -> Vec<Vec<(f64, f64)>> {
    let grid = |vals: Vec<f64>| -> Vec<f64> {
        let mut v = vals;
        v.sort_by(f64::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        v
    };
    let xs = grid(rects.iter().flat_map(|r| [r.0, r.1]).collect());
    let ys = grid(rects.iter().flat_map(|r| [r.2, r.3]).collect());
    let (nx, ny) = (xs.len(), ys.len());
    if nx < 2 || ny < 2 {
        return Vec::new();
    }
    let at = |v: &[f64], x: f64| v.partition_point(|a| *a < x - 1e-6);
    let mut on = vec![false; (nx - 1) * (ny - 1)];
    for r in rects {
        for i in at(&xs, r.0)..at(&xs, r.1) {
            for j in at(&ys, r.2)..at(&ys, r.3) {
                on[i * (ny - 1) + j] = true;
            }
        }
    }
    let cell = |i: isize, j: isize| -> bool {
        i >= 0 && j >= 0 && (i as usize) < nx - 1 && (j as usize) < ny - 1 && on[i as usize * (ny - 1) + j as usize]
    };
    // Boundary edges, interior on the left, between grid nodes (i, j).
    let mut next: std::collections::HashMap<(usize, usize), Vec<(usize, usize)>> = Default::default();
    for i in 0..nx - 1 {
        for j in 0..ny - 1 {
            if !on[i * (ny - 1) + j] {
                continue;
            }
            let (a, b) = (i as isize, j as isize);
            if !cell(a, b - 1) {
                next.entry((i, j)).or_default().push((i + 1, j));
            }
            if !cell(a + 1, b) {
                next.entry((i + 1, j)).or_default().push((i + 1, j + 1));
            }
            if !cell(a, b + 1) {
                next.entry((i + 1, j + 1)).or_default().push((i, j + 1));
            }
            if !cell(a - 1, b) {
                next.entry((i, j + 1)).or_default().push((i, j));
            }
        }
    }
    let mut starts: Vec<(usize, usize)> = next.keys().copied().collect();
    starts.sort();
    let mut rings = Vec::new();
    for s in starts {
        while next.get(&s).is_some_and(|v| !v.is_empty()) {
            let mut ring = vec![s];
            let mut cur = s;
            loop {
                let Some(list) = next.get_mut(&cur) else { break };
                let Some(nxt) = list.pop() else { break };
                if nxt == s {
                    break;
                }
                ring.push(nxt);
                cur = nxt;
            }
            let pts: Vec<(f64, f64)> = ring.iter().map(|&(i, j)| (xs[i], ys[j])).collect();
            let area: f64 = (0..pts.len())
                .map(|k| {
                    let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
                    a.0 * b.1 - b.0 * a.1
                })
                .sum();
            if area > 1e-6 && pts.len() >= 3 {
                rings.push(pts);
            }
        }
    }
    rings
}

/// Straight lines in place of steps, around a whole ring (anticlockwise, not
/// closed): from each corner, the longest straight line to a later corner
/// that keeps every corner in between inside (on its left), no more than
/// `STRAIGHTEN` from it, so that real notches stay, and at least `CLEAR`
/// from any other block's pile, none of which may end up inside.
fn straighten_ring(ring: &[(f64, f64)], others: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let n = ring.len();
    if n < 4 {
        let mut r = ring.to_vec();
        r.push(ring[0]);
        return r;
    }
    let p = |k: usize| ring[k % n];
    let valid = |a: usize, b: usize| -> bool {
        let (pa, pb) = (p(a), p(b));
        let (dx, dy) = (pb.0 - pa.0, pb.1 - pa.1);
        let len = dx.hypot(dy);
        if len < 1e-9 {
            return false;
        }
        let side = |q: (f64, f64)| (dx * (q.1 - pa.1) - dy * (q.0 - pa.0)) / len;
        let chain: Vec<(f64, f64)> = (a..=b).map(p).collect();
        if !chain[1..chain.len() - 1].iter().all(|&q| (-1e-6..=STRAIGHTEN).contains(&side(q))) {
            return false;
        }
        // The area the line adds must hold no other block's pile, and the
        // line keeps clear of them.
        let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for q in &chain {
            (x0, x1, y0, y1) = (x0.min(q.0), x1.max(q.0), y0.min(q.1), y1.max(q.1));
        }
        others
            .iter()
            .filter(|o| o.0 > x0 - CLEAR && o.0 < x1 + CLEAR && o.1 > y0 - CLEAR && o.1 < y1 + CLEAR)
            .all(|&o| {
                let t = (((o.0 - pa.0) * dx + (o.1 - pa.1) * dy) / (len * len)).clamp(0.0, 1.0);
                let dist = (o.0 - pa.0 - t * dx).hypot(o.1 - pa.1 - t * dy);
                dist >= CLEAR && !in_polygon(&chain, o)
            })
    };
    // Start at the corner that begins the longest side: a real corner.
    let start = (0..n)
        .max_by(|&a, &b| {
            let l = |k: usize| (p(k + 1).0 - p(k).0).hypot(p(k + 1).1 - p(k).1);
            l(a).total_cmp(&l(b))
        })
        .unwrap_or(0);
    let mut kept = Vec::new();
    let mut a = start;
    while a < start + n {
        kept.push(p(a));
        // The farthest corner a straight line can reach (looking a bounded
        // number of corners ahead).
        let reach = (start + n - a).min(LOOK_AHEAD);
        let step = (2..=reach).rev().find(|&len| valid(a, a + len)).unwrap_or(1);
        a += step;
    }
    kept.push(p(start));
    kept
}

/// Is `q` inside the closed polygon `poly` (even-odd rule)?
fn in_polygon(poly: &[(f64, f64)], q: (f64, f64)) -> bool {
    let mut c = false;
    let n = poly.len();
    for k in 0..n {
        let (a, b) = (poly[k], poly[(k + 1) % n]);
        if (a.1 > q.1) != (b.1 > q.1) && q.0 < (b.0 - a.0) * (q.1 - a.1) / (b.1 - a.1) + a.0 {
            c = !c;
        }
    }
    c
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

    #[test]
    fn a_block_goes_around_another_block_in_line_with_its_own_row() {
        // Block A: five full rows, except the third, which has a gap filled
        // by a row of block B (as GESF block 5 around block 6's first row).
        let mut a = trackers(0.0, 5, |_| 0.0, 20); // 152 m long rows
        let b_row: Vec<(f64, f64)> = (0..6).map(|j| (10.8, 60.0 + 8.0 * j as f64)).collect();
        a[2] = (0..6).map(|j| (10.8, 8.0 * j as f64)).collect(); // south of B
        a.push((0..6).map(|j| (10.8, 112.0 + 8.0 * j as f64)).collect()); // north of B
        // East of it, block A only covers the north, block B the rest.
        for (k, u) in [(3, 16.2), (4, 21.6)] {
            a[k] = (0..6).map(|j| (u, 112.0 + 8.0 * j as f64)).collect();
        }
        let mut others = b_row.clone();
        for u in [16.2, 21.6] {
            others.extend((0..12).map(|j| (u, 8.0 * j as f64)));
        }
        let rings = outline_among(&a, &others);
        assert!(a.iter().flatten().all(|&p| rings.iter().any(|r| inside(r, p))));
        assert!(
            others.iter().all(|&p| rings.iter().all(|r| !inside(r, p))),
            "block B stays outside: {rings:?}"
        );
    }
}
