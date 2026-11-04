//! Set-out: pile positions that keep the design length of every torque-tube
//! span once the tops of pile are known.
//!
//! The layout gives each tracker's piles in plan, with the distance between
//! two consecutive piles equal to the length of the torque-tube span between
//! them (the layout is drawn flat). When the two tops differ in height by `dz`,
//! that span covers only `sqrt(L^2 - dz^2)` in plan, so the piles must move
//! along the tracker axis. The anchor pile (the motor pile) keeps its design
//! position; the others slide towards it, by a few millimetres per span on a
//! sloping tracker.

use crate::model::Tracker;

/// New positions of a tracker's piles.
#[derive(Clone, Debug)]
pub struct SetOut {
    /// Index of the pile kept at its design position.
    pub anchor: usize,
    /// The anchor is the tracker's single motor pile (else: the middle pile).
    pub anchor_is_motor: bool,
    pub easting: Vec<f64>,
    pub northing: Vec<f64>,
    /// Signed move along the tracker from the design position (m), positive
    /// towards the last pile (increasing Northing for a north-south tracker).
    pub shift: Vec<f64>,
}

/// A motor pile: a type code whose letters end in `MP` (`HMP-20`, `SMP-31`),
/// or a description containing "motor" (`Heavy Motor Pier`).
pub fn is_motor_pile(pile_type: &str) -> bool {
    let up = pile_type.trim().to_uppercase();
    let letters: String = up.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    letters.ends_with("MP") || up.contains("MOTOR")
}

/// The pile kept fixed: the tracker's motor pile if it has exactly one, else
/// the middle pile.
pub fn anchor_index(tracker: &Tracker) -> (usize, bool) {
    let motors: Vec<usize> = tracker
        .piles
        .iter()
        .enumerate()
        .filter(|(_, p)| is_motor_pile(&p.pile_type))
        .map(|(i, _)| i)
        .collect();
    match motors.as_slice() {
        [m] => (*m, true),
        _ => (tracker.piles.len() / 2, false),
    }
}

/// Positions keeping each span's design length (its plan length in the
/// layout) between the given tops.
pub fn set_out(tracker: &Tracker, tops: &[f64]) -> SetOut {
    let piles = &tracker.piles;
    let n = piles.len();
    let (anchor, anchor_is_motor) = anchor_index(tracker);
    if n < 2 {
        return SetOut {
            anchor,
            anchor_is_motor,
            easting: piles.iter().map(|p| p.easting).collect(),
            northing: piles.iter().map(|p| p.northing).collect(),
            shift: vec![0.0; n],
        };
    }
    let design = tracker.chainage();
    let (dx, dy) = (
        piles[n - 1].easting - piles[0].easting,
        piles[n - 1].northing - piles[0].northing,
    );
    let len = dx.hypot(dy);
    let (ux, uy) = if len > 0.0 {
        (dx / len, dy / len)
    } else {
        (0.0, 1.0)
    };

    let plan = |i: usize| {
        // Span i -> i+1: design length along the tube, height difference of the tops.
        let l = design[i + 1] - design[i];
        let dz = tops[i + 1] - tops[i];
        (l * l - dz * dz).max(0.0).sqrt()
    };
    let mut s = vec![0.0; n];
    s[anchor] = design[anchor];
    for i in anchor..n - 1 {
        s[i + 1] = s[i] + plan(i);
    }
    for i in (1..=anchor).rev() {
        s[i - 1] = s[i] - plan(i - 1);
    }
    let shift: Vec<f64> = s.iter().zip(&design).map(|(a, b)| a - b).collect();
    SetOut {
        anchor,
        anchor_is_motor,
        easting: piles
            .iter()
            .zip(&shift)
            .map(|(p, d)| p.easting + ux * d)
            .collect(),
        northing: piles
            .iter()
            .zip(&shift)
            .map(|(p, d)| p.northing + uy * d)
            .collect(),
        shift,
    }
}

/// Largest difference (m) between a span's length along the tube, from the
/// set-out positions and tops, and its design length.
pub fn span_error(tracker: &Tracker, so: &SetOut, tops: &[f64]) -> f64 {
    let design = tracker.chainage();
    (0..tracker.piles.len().saturating_sub(1))
        .map(|i| {
            let plan =
                (so.easting[i + 1] - so.easting[i]).hypot(so.northing[i + 1] - so.northing[i]);
            let along = plan.hypot(tops[i + 1] - tops[i]);
            (along - (design[i + 1] - design[i])).abs()
        })
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Pile;

    fn tracker(types: &[&str], spacing: f64) -> Tracker {
        Tracker {
            id: "T".into(),
            piles: types
                .iter()
                .enumerate()
                .map(|(i, t)| Pile {
                    name: format!("P{i}"),
                    tracker: "T".into(),
                    block: String::new(),
                    section: String::new(),
                    pile_type: t.to_string(),
                    easting: 1000.0,
                    northing: 5000.0 + spacing * i as f64,
                    z_lidar: 0.0,
                    min_top: None,
                })
                .collect(),
        }
    }

    #[test]
    fn motor_piles_are_recognised() {
        for t in ["HMP-20", "SMP-31", "Heavy Motor Pier", "smp20"] {
            assert!(is_motor_pile(t), "{t}");
        }
        for t in ["HAP-20", "SAP-7", "SAPE-15", "Standard Array Pier", ""] {
            assert!(!is_motor_pile(t), "{t}");
        }
    }

    #[test]
    fn flat_tops_keep_design_positions() {
        let tr = tracker(&["SAP-7", "SAP-7", "HMP-20", "SAP-7", "SAP-7"], 9.415);
        let so = set_out(&tr, &[10.0; 5]);
        assert_eq!((so.anchor, so.anchor_is_motor), (2, true));
        assert!(so.shift.iter().all(|d| d.abs() < 1e-12));
    }

    #[test]
    fn sloping_tops_pull_piles_towards_the_motor_pile() {
        // 5 % slope on 9.415 m spans: each span is 11.8 mm shorter in plan.
        let tr = tracker(&["SAP-7", "SAP-7", "HMP-20", "SAP-7", "SAP-7"], 9.415);
        let tops: Vec<f64> = (0..5).map(|i| 10.0 + 0.05 * 9.415 * i as f64).collect();
        let so = set_out(&tr, &tops);
        let per_span = 9.415 - (9.415f64.powi(2) - (0.05 * 9.415f64).powi(2)).sqrt();
        assert!((so.shift[2]).abs() < 1e-12);
        assert!(
            (so.shift[3] + per_span).abs() < 1e-9 && (so.shift[4] + 2.0 * per_span).abs() < 1e-9
        );
        assert!(
            (so.shift[1] - per_span).abs() < 1e-9 && (so.shift[0] - 2.0 * per_span).abs() < 1e-9
        );
        assert!(so.easting.iter().all(|e| *e == 1000.0));
        assert!(span_error(&tr, &so, &tops) < 1e-9);
    }

    #[test]
    fn no_single_motor_pile_anchors_the_middle() {
        let tr = tracker(&["SAP-7"; 5], 8.0);
        assert_eq!(anchor_index(&tr), (2, false));
        let tr = tracker(&["HMP-20", "SAP-7", "SAP-7", "SAP-7", "HMP-20"], 8.0);
        assert_eq!(anchor_index(&tr), (2, false));
    }
}
