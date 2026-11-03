//! Data model: piles and trackers.

use std::collections::BTreeMap;

/// One pile position with its LIDAR ground elevation.
#[derive(Clone, Debug, PartialEq)]
pub struct Pile {
    pub name: String,
    pub tracker: String,
    pub block: String,
    pub section: String,
    pub pile_type: String,
    pub easting: f64,
    pub northing: f64,
    /// Ground elevation from the LIDAR survey (m).
    pub z_lidar: f64,
}

/// All piles of one tracker, ordered along the tracker (increasing Northing).
#[derive(Clone, Debug)]
pub struct Tracker {
    pub id: String,
    pub piles: Vec<Pile>,
}

impl Tracker {
    /// Horizontal distance along the tracker from the first pile, one value per pile.
    ///
    /// Uses the planar distance between consecutive piles. For a tracker whose
    /// piles share the same Easting this equals the difference in Northing,
    /// which is what the original Python script uses.
    pub fn chainage(&self) -> Vec<f64> {
        let mut s = Vec::with_capacity(self.piles.len());
        let mut acc = 0.0;
        for (i, p) in self.piles.iter().enumerate() {
            if i > 0 {
                let q = &self.piles[i - 1];
                acc += (p.easting - q.easting).hypot(p.northing - q.northing);
            }
            s.push(acc);
        }
        s
    }
}

/// Normalise a block label so that `"01"`, `"1"` and `1.0` compare equal.
pub fn normalize_block(raw: &str) -> String {
    let t = raw.trim();
    match t.parse::<f64>() {
        Ok(v) if v.is_finite() && v.fract() == 0.0 => format!("{}", v as i64),
        _ => t.to_string(),
    }
}

/// Group piles by tracker id, each group sorted by Northing. Trackers come out
/// in alphabetical order of id, so results are deterministic.
pub fn group_trackers(piles: Vec<Pile>) -> Vec<Tracker> {
    let mut map: BTreeMap<String, Vec<Pile>> = BTreeMap::new();
    for p in piles {
        map.entry(p.tracker.clone()).or_default().push(p);
    }
    map.into_iter()
        .map(|(id, mut piles)| {
            piles.sort_by(|a, b| a.northing.total_cmp(&b.northing));
            Tracker { id, piles }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pile(name: &str, tracker: &str, n: f64, z: f64) -> Pile {
        Pile {
            name: name.into(),
            tracker: tracker.into(),
            block: "1".into(),
            section: "1".into(),
            pile_type: String::new(),
            easting: 100.0,
            northing: n,
            z_lidar: z,
        }
    }

    #[test]
    fn groups_and_sorts_by_northing() {
        let piles = vec![
            pile("b", "T2", 5.0, 0.0),
            pile("a2", "T1", 20.0, 0.0),
            pile("a1", "T1", 10.0, 0.0),
        ];
        let t = group_trackers(piles);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].id, "T1");
        assert_eq!(t[0].piles[0].name, "a1");
        assert_eq!(t[0].chainage(), vec![0.0, 10.0]);
    }

    #[test]
    fn block_labels_compare_numerically() {
        assert_eq!(normalize_block("01"), "1");
        assert_eq!(normalize_block(" 1.0 "), "1");
        assert_eq!(normalize_block("1"), "1");
        assert_eq!(normalize_block("B2"), "B2");
    }
}
