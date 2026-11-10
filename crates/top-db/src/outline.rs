//! Block outlines: a line around all the piles of a block that follows its
//! shape (notches, steps), unlike a convex hull.
//!
//! On a grid of 1 m cells:
//! 1. every cell within `CLOSE` (8 m) of a pile is filled (this bridges the
//!    gaps between rows and trackers of the block);
//! 2. the filled area is shrunk back by `CLOSE - MARGIN`, so its edge passes
//!    about `MARGIN` outside the outer piles (a morphological *closing*);
//! 3. the edge between filled and empty cells is traced into closed rings,
//!    which are then simplified (Douglas-Peucker).
//!
//! Distances use an exact Euclidean distance transform (Felzenszwalb &
//! Huttenlocher), so the whole site takes a fraction of a second.

use crate::store::{State, natural_key};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// Cell size (m).
const CELL: f64 = 1.0;
/// Gaps narrower than twice this (m) inside a block are bridged. A block
/// still in several parts (an access road through it) is outlined again with
/// the next, wider values, until it is one line.
const CLOSE: [f64; 8] = [8.0, 12.0, 16.0, 20.0, 25.0, 30.0, 40.0, 50.0];
/// The outline passes about this far (m) outside the outer piles.
const MARGIN: f64 = 2.0;
/// Simplification tolerance (m).
const SIMPLIFY: f64 = 1.0;

#[derive(Clone, Debug, Serialize)]
pub struct Outline {
    pub block: String,
    pub piles: usize,
    /// Closed rings (first point repeated at the end), Easting / Northing.
    /// Usually one; several when a block is in separate parts.
    pub rings: Vec<Vec<(f64, f64)>>,
}

/// Outline of every block of the master list (design positions).
pub fn block_outlines(state: &State) -> Vec<Outline> {
    let mut by_block: BTreeMap<&str, Vec<(f64, f64)>> = BTreeMap::new();
    for p in &state.piles {
        by_block
            .entry(p.block.as_str())
            .or_default()
            .push((p.easting, p.northing));
    }
    let mut out: Vec<Outline> = by_block
        .into_iter()
        .map(|(block, pts)| Outline {
            block: block.to_string(),
            piles: pts.len(),
            rings: outline(&pts),
        })
        .collect();
    out.sort_by_key(|o| natural_key(&o.block));
    out
}

/// Rings around a set of points (see the module documentation): one when
/// the parts are less than 100 m apart.
pub fn outline(points: &[(f64, f64)]) -> Vec<Vec<(f64, f64)>> {
    let mut rings = Vec::new();
    for close in CLOSE {
        rings = outline_with(points, close);
        if rings.len() <= 1 {
            break;
        }
    }
    rings
}

fn outline_with(points: &[(f64, f64)], close: f64) -> Vec<Vec<(f64, f64)>> {
    if points.is_empty() {
        return Vec::new();
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in points {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let pad = close + MARGIN + 3.0 * CELL;
    let (ox, oy) = (x0 - pad, y0 - pad);
    let w = ((x1 - x0 + 2.0 * pad) / CELL).ceil() as usize + 1;
    let h = ((y1 - y0 + 2.0 * pad) / CELL).ceil() as usize + 1;

    // 1. Within CLOSE of a pile.
    let mut seeds = vec![false; w * h];
    for &(x, y) in points {
        let i = ((x - ox) / CELL) as usize;
        let j = ((y - oy) / CELL) as usize;
        seeds[j * w + i] = true;
    }
    let near = edt(&seeds, w, h);
    let r1 = (close / CELL).powi(2);
    let grown: Vec<bool> = near.iter().map(|&d| d <= r1).collect();
    // 2. Shrink back: keep cells further than CLOSE - MARGIN from outside.
    let outside: Vec<bool> = grown.iter().map(|g| !g).collect();
    let inside = edt(&outside, w, h);
    let r2 = ((close - MARGIN) / CELL).powi(2);
    let filled: Vec<bool> = inside.iter().map(|&d| d > r2).collect();

    // 3. Directed edges with the filled cell on the left, then rings.
    let at = |i: isize, j: isize| -> bool {
        i >= 0
            && j >= 0
            && (i as usize) < w
            && (j as usize) < h
            && filled[j as usize * w + i as usize]
    };
    let mut next: HashMap<(isize, isize), Vec<(isize, isize)>> = HashMap::new();
    let mut add = |a: (isize, isize), b: (isize, isize)| next.entry(a).or_default().push(b);
    for j in 0..h as isize {
        for i in 0..w as isize {
            if !at(i, j) {
                continue;
            }
            if !at(i, j - 1) {
                add((i, j), (i + 1, j));
            }
            if !at(i + 1, j) {
                add((i + 1, j), (i + 1, j + 1));
            }
            if !at(i, j + 1) {
                add((i + 1, j + 1), (i, j + 1));
            }
            if !at(i - 1, j) {
                add((i, j + 1), (i, j));
            }
        }
    }
    let mut starts: Vec<(isize, isize)> = next.keys().copied().collect();
    starts.sort();
    let mut rings = Vec::new();
    for s in starts {
        while next.get(&s).is_some_and(|v| !v.is_empty()) {
            let mut ring = vec![s];
            let mut prev_dir = (0, 0);
            let mut cur = s;
            loop {
                let outs = next.get_mut(&cur).expect("traced edge");
                // At a vertex with two ways out (cells touching by a corner),
                // turn left first: each part gets its own ring.
                let k = if outs.len() > 1 {
                    let left = (-prev_dir.1, prev_dir.0);
                    outs.iter()
                        .position(|&n| (n.0 - cur.0, n.1 - cur.1) == left)
                        .unwrap_or(0)
                } else {
                    0
                };
                let n = outs.swap_remove(k);
                prev_dir = (n.0 - cur.0, n.1 - cur.1);
                cur = n;
                ring.push(cur);
                if cur == s {
                    break;
                }
            }
            let pts: Vec<(f64, f64)> = ring
                .iter()
                .map(|&(i, j)| (ox + i as f64 * CELL, oy + j as f64 * CELL))
                .collect();
            // Outer rings run anticlockwise (positive area); holes are dropped.
            if area(&pts) > 4.0 * CELL * CELL {
                rings.push(simplify(&pts));
            }
        }
    }
    rings
}

/// Signed area (positive anticlockwise) of a closed ring.
fn area(ring: &[(f64, f64)]) -> f64 {
    ring.windows(2)
        .map(|w| w[0].0 * w[1].1 - w[1].0 * w[0].1)
        .sum::<f64>()
        / 2.0
}

/// Squared Euclidean distance (in cells) from every cell to the nearest seed.
fn edt(seeds: &[bool], w: usize, h: usize) -> Vec<f64> {
    const INF: f64 = 1e20;
    let mut d: Vec<f64> = seeds.iter().map(|&s| if s { 0.0 } else { INF }).collect();
    let n = w.max(h);
    let (mut f, mut out, mut v, mut z) = (
        vec![0.0; n],
        vec![0.0; n],
        vec![0usize; n],
        vec![0.0; n + 1],
    );
    for i in 0..w {
        for j in 0..h {
            f[j] = d[j * w + i];
        }
        dt1d(&f[..h], &mut out[..h], &mut v, &mut z);
        for j in 0..h {
            d[j * w + i] = out[j];
        }
    }
    for j in 0..h {
        f[..w].copy_from_slice(&d[j * w..(j + 1) * w]);
        dt1d(&f[..w], &mut out[..w], &mut v, &mut z);
        d[j * w..(j + 1) * w].copy_from_slice(&out[..w]);
    }
    d
}

/// One-dimensional squared distance transform (lower envelope of parabolas).
fn dt1d(f: &[f64], out: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    let s = |q: usize, p: usize| {
        ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64))
    };
    let mut k = 0;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let mut x = s(q, v[k]);
        while x <= z[k] {
            k -= 1;
            x = s(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = x;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        *o = (q as f64 - p as f64).powi(2) + f[p];
    }
}

/// Douglas-Peucker simplification of a closed ring, kept closed, rounded to
/// the centimetre.
fn simplify(ring: &[(f64, f64)]) -> Vec<(f64, f64)> {
    fn dp(pts: &[(f64, f64)], tol: f64, keep: &mut [bool], a: usize, b: usize) {
        if b <= a + 1 {
            return;
        }
        let (ax, ay) = pts[a];
        let (bx, by) = pts[b];
        let len = (bx - ax).hypot(by - ay);
        let (mut far, mut dmax) = (a, 0.0);
        for (i, &(px, py)) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = if len < 1e-12 {
                (px - ax).hypot(py - ay)
            } else {
                ((bx - ax) * (ay - py) - (ax - px) * (by - ay)).abs() / len
            };
            if d > dmax {
                (far, dmax) = (i, d);
            }
        }
        if dmax > tol {
            keep[far] = true;
            dp(pts, tol, keep, a, far);
            dp(pts, tol, keep, far, b);
        }
    }
    let n = ring.len();
    if n < 5 {
        return ring.to_vec();
    }
    // Split the ring at the point furthest from its start, and simplify both halves.
    let far = (1..n - 1)
        .max_by(|&i, &j| {
            let d = |k: usize| (ring[k].0 - ring[0].0).hypot(ring[k].1 - ring[0].1);
            d(i).total_cmp(&d(j))
        })
        .unwrap_or(n / 2);
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[far] = true;
    keep[n - 1] = true;
    dp(ring, SIMPLIFY, &mut keep, 0, far);
    dp(ring, SIMPLIFY, &mut keep, far, n - 1);
    ring.iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(&(x, y), _)| ((x * 100.0).round() / 100.0, (y * 100.0).round() / 100.0))
        .collect()
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

    fn grid(cols: usize, rows: usize, dx: f64, dy: f64, x0: f64, y0: f64) -> Vec<(f64, f64)> {
        (0..cols)
            .flat_map(|i| (0..rows).map(move |j| (x0 + i as f64 * dx, y0 + j as f64 * dy)))
            .collect()
    }

    #[test]
    fn one_ring_around_every_pile_following_the_shape() {
        // An L-shaped block: 20 rows 5.4 m apart, piles 8 m apart; the right
        // half is only half as long (a notch a convex hull would cut across).
        let mut pts = grid(10, 30, 5.4, 8.0, 305_000.0, 6_428_000.0);
        pts.extend(grid(10, 15, 5.4, 8.0, 305_054.0, 6_428_000.0));
        let rings = outline(&pts);
        assert_eq!(rings.len(), 1, "one ring");
        let r = &rings[0];
        assert_eq!(r.first(), r.last(), "closed");
        assert!(area(r) > 0.0);
        assert!(pts.iter().all(|&p| inside(r, p)), "every pile inside");
        // The notch stays outside: a point in the empty upper-right part.
        assert!(!inside(r, (305_080.0, 6_428_200.0)));
        // The line passes about MARGIN outside the outer piles.
        let below = (305_020.0, 6_428_000.0 - MARGIN - 1.0);
        assert!(!inside(r, below) && inside(r, (305_020.0, 6_428_000.0 - MARGIN + 1.0)));
        assert!(r.len() < 200, "simplified: {} points", r.len());
    }

    #[test]
    fn a_block_in_two_parts_gets_one_line() {
        // Two halves 35 m apart (an access road): one ring around both.
        let mut pts = grid(5, 10, 5.4, 8.0, 1000.0, 1000.0);
        pts.extend(grid(5, 10, 5.4, 8.0, 1000.0, 1107.0));
        let rings = outline(&pts);
        assert_eq!(rings.len(), 1);
        assert!(pts.iter().all(|&p| inside(&rings[0], p)));
        // Far apart (over 100 m): one line around each.
        let mut far = grid(5, 10, 5.4, 8.0, 1000.0, 1000.0);
        far.extend(grid(5, 10, 5.4, 8.0, 1250.0, 1000.0));
        let rings = outline(&far);
        assert_eq!(rings.len(), 2);
        assert!(far.iter().all(|&p| rings.iter().any(|r| inside(r, p))));
        assert!(outline(&[]).is_empty());
        assert_eq!(outline(&[(5.0, 5.0)]).len(), 1);
    }
}
