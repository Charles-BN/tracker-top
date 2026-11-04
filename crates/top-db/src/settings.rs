//! Project settings stored in the database: optimisation parameters and
//! survey-matching tolerances.

use serde::{Deserialize, Serialize};
use top_core::Params;

/// What a pile's flood value means.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FloodMode {
    /// Water depth above the ground at the pile (m): water level = ground + value.
    #[default]
    Depth,
    /// Water surface elevation (m, same datum as the ground elevations).
    Elevation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Nominal reveal above ground (m).
    pub reveal_nominal: f64,
    /// Lowest reveal = nominal + this (m, negative).
    pub delta_min: f64,
    /// Highest reveal = nominal + this (m).
    pub delta_max: f64,
    /// Max angle change between two consecutive spans (deg).
    pub max_angle_deg: f64,
    /// Cap on cut/fill at any pile (m).
    pub max_cut_fill: f64,
    /// Cut/fill above this (m) marks a tracker as Graded.
    pub grading_threshold: f64,

    /// A survey point whose ID is not in the master list is matched to the
    /// nearest pile if it lies within this distance (m).
    pub match_radius_m: f64,
    /// Warn when a surveyed point is further than this from its design position (m).
    pub offset_warn_m: f64,
    /// Reject a survey point further than this from its design position (m):
    /// almost always a wrong point ID.
    pub offset_reject_m: f64,
    /// Warn when a new elevation differs from the previous one by more than this (m).
    pub jump_warn_m: f64,

    /// How flood values are read.
    pub flood_mode: FloodMode,
    /// Required height of the top of pile above the flood water level (m).
    /// The lowest allowed top of a pile is `water level + flood_clearance_m`.
    pub flood_clearance_m: f64,
}

impl Default for Settings {
    fn default() -> Self {
        let p = Params::default();
        Self {
            reveal_nominal: p.reveal_nominal,
            delta_min: p.delta_min,
            delta_max: p.delta_max,
            max_angle_deg: p.max_angle_deg,
            max_cut_fill: p.max_cut_fill,
            grading_threshold: p.grading_threshold,
            match_radius_m: 0.5,
            offset_warn_m: 0.10,
            offset_reject_m: 1.0,
            jump_warn_m: 0.30,
            flood_mode: FloodMode::Depth,
            flood_clearance_m: 0.0,
        }
    }
}

impl Settings {
    pub fn params(&self) -> Params {
        Params {
            reveal_nominal: self.reveal_nominal,
            delta_min: self.delta_min,
            delta_max: self.delta_max,
            max_angle_deg: self.max_angle_deg,
            max_cut_fill: self.max_cut_fill,
            grading_threshold: self.grading_threshold,
            ..Params::default()
        }
    }

    /// The parameters that change optimisation results, as stable JSON. Part of
    /// every tracker's input signature, so editing them re-runs every tracker.
    pub fn params_json(&self) -> String {
        serde_json::json!({
            "reveal_nominal": self.reveal_nominal,
            "delta_min": self.delta_min,
            "delta_max": self.delta_max,
            "max_angle_deg": self.max_angle_deg,
            "max_cut_fill": self.max_cut_fill,
            "grading_threshold": self.grading_threshold,
            "flood_mode": self.flood_mode,
            "flood_clearance_m": self.flood_clearance_m,
        })
        .to_string()
    }

    /// Flood water level at a pile (m, absolute), from its flood value and the
    /// ground elevation in use.
    pub fn flood_water_level(&self, ground: f64, flood: Option<f64>) -> Option<f64> {
        let v = flood.filter(|v| v.is_finite())?;
        Some(match self.flood_mode {
            FloodMode::Depth => ground + v,
            FloodMode::Elevation => v,
        })
    }

    /// Lowest allowed top of pile: flood water level + clearance.
    pub fn min_top(&self, ground: f64, flood: Option<f64>) -> Option<f64> {
        self.flood_water_level(ground, flood)
            .map(|w| w + self.flood_clearance_m)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.params().validate()?;
        let pos = [
            ("match_radius_m", self.match_radius_m),
            ("offset_warn_m", self.offset_warn_m),
            ("offset_reject_m", self.offset_reject_m),
            ("jump_warn_m", self.jump_warn_m),
            ("reveal_nominal", self.reveal_nominal),
        ];
        for (name, v) in pos {
            if !(v.is_finite() && v >= 0.0) {
                return Err(format!("{name} must be a number >= 0"));
            }
        }
        if !self.flood_clearance_m.is_finite() || self.flood_clearance_m.abs() > 20.0 {
            return Err("flood_clearance_m must be a number between -20 and 20".into());
        }
        if self.offset_warn_m > self.offset_reject_m {
            return Err("offset_warn_m must not exceed offset_reject_m".into());
        }
        Ok(())
    }
}
