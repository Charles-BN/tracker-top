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

    /// How flood values are read: water levels (default; independent of the
    /// ground, which moves away from the LIDAR the flood study used) or depths.
    pub flood_mode: FloodMode,
    /// Flood rule: at the reference pile height (m above ground) the lowest
    /// module edge is `flood_module_clearance_m` above the ground, and it must
    /// stay `flood_freeboard_m` above the flood level. So the top of a pile
    /// must be at least flood level + freeboard + (pile height - module
    /// clearance): with 1.075, 0.30 and 0.10, flood level + 0.875 m. Piles
    /// rise only where the water is deeper than module clearance - freeboard
    /// (0.20 m) below that reference height.
    pub flood_pile_height_m: f64,
    pub flood_module_clearance_m: f64,
    pub flood_freeboard_m: f64,

    /// Coordinate system of Easting/Northing (EPSG code), written into the
    /// GeoPackage export for ArcGIS. Default 7856, GDA2020 / MGA zone 56.
    pub epsg: Option<u32>,

    /// Keep each torque-tube span at its design length: piles are set out along
    /// the tracker from the motor pile, and the tracker re-checked there.
    pub fixed_spans: bool,
    /// Flag piles whose set-out position moves more than this from design (m).
    pub shift_warn_m: f64,
    /// Warn on a tracker whose torque-tube slope (mean of the absolute slopes
    /// of its spans between the tops of pile, %) is above this.
    pub slope_warn_pct: f64,
    /// Calculated values (tops, cut/fill, set-out positions) are rounded to
    /// this step (m): 0.01 = the centimetre; 0 keeps them exact. The limits
    /// are checked on the rounded values.
    pub round_step: f64,

    /// Send a block's file to the installers' folder once every pile in it is
    /// surveyed and every tracker has a current result (needs a shared folder).
    pub auto_export: bool,
    /// Folder for those files, inside the shared folder ("/" for subfolders).
    pub auto_export_folder: String,
    /// Export profile (columns) used for those files.
    pub auto_export_profile: String,
    /// Wait until every tracker of the block is Compliant or Graded.
    pub auto_export_require_ok: bool,
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
            flood_mode: FloodMode::Elevation,
            flood_pile_height_m: 1.075,
            flood_module_clearance_m: 0.30,
            flood_freeboard_m: 0.10,
            epsg: Some(7856),
            fixed_spans: true,
            shift_warn_m: 0.10,
            slope_warn_pct: 6.0,
            round_step: 0.01,
            auto_export: true,
            auto_export_folder: "For installers".into(),
            auto_export_profile: "Installers".into(),
            auto_export_require_ok: true,
        }
    }
}

impl Settings {
    /// Decimals to show calculated values with: 2 when rounded to the
    /// centimetre, 3 to the millimetre, else 4.
    pub fn decimals(&self) -> usize {
        if self.round_step >= 0.0095 {
            2
        } else if self.round_step >= 0.00095 {
            3
        } else {
            4
        }
    }

    pub fn params(&self) -> Params {
        Params {
            reveal_nominal: self.reveal_nominal,
            delta_min: self.delta_min,
            delta_max: self.delta_max,
            max_angle_deg: self.max_angle_deg,
            max_cut_fill: self.max_cut_fill,
            grading_threshold: self.grading_threshold,
            fixed_spans: self.fixed_spans,
            round_step: self.round_step,
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
            "flood_pile_height_m": self.flood_pile_height_m,
            "flood_module_clearance_m": self.flood_module_clearance_m,
            "flood_freeboard_m": self.flood_freeboard_m,
            "fixed_spans": self.fixed_spans,
            "round_step": self.round_step,
            // Bumped when the calculation itself changes, so results are redone.
            "method": 3,
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

    /// Height (m) the top of a pile must keep above the flood level: the
    /// freeboard, plus the drop from the top of pile to the lowest module edge.
    pub fn flood_clearance(&self) -> f64 {
        self.flood_freeboard_m + self.flood_pile_height_m - self.flood_module_clearance_m
    }

    /// Lowest allowed top of pile: flood water level + `flood_clearance()`.
    pub fn min_top(&self, ground: f64, flood: Option<f64>) -> Option<f64> {
        self.flood_water_level(ground, flood)
            .map(|w| w + self.flood_clearance())
    }

    pub fn validate(&self) -> Result<(), String> {
        self.params().validate()?;
        let pos = [
            ("match_radius_m", self.match_radius_m),
            ("offset_warn_m", self.offset_warn_m),
            ("offset_reject_m", self.offset_reject_m),
            ("jump_warn_m", self.jump_warn_m),
            ("shift_warn_m", self.shift_warn_m),
            ("slope_warn_pct", self.slope_warn_pct),
            ("reveal_nominal", self.reveal_nominal),
        ];
        for (name, v) in pos {
            if !(v.is_finite() && v >= 0.0) {
                return Err(format!("{name} must be a number >= 0"));
            }
        }
        if !(self.round_step.is_finite() && (0.0..=0.05).contains(&self.round_step)) {
            return Err("round_step must be between 0 and 0.05 m (0.01 = centimetre)".into());
        }
        for (name, v) in [
            ("flood_pile_height_m", self.flood_pile_height_m),
            ("flood_module_clearance_m", self.flood_module_clearance_m),
            ("flood_freeboard_m", self.flood_freeboard_m),
        ] {
            if !(v.is_finite() && (0.0..=10.0).contains(&v)) {
                return Err(format!("{name} must be a number between 0 and 10 m"));
            }
        }
        if self.flood_module_clearance_m > self.flood_pile_height_m {
            return Err(
                "the module clearance cannot exceed the pile height it is measured at".into(),
            );
        }
        if self.epsg.is_some_and(|c| !(1024..=999_999).contains(&c)) {
            return Err(
                "the EPSG code must be a number such as 7856 (GDA2020 / MGA zone 56)".into(),
            );
        }
        let folder = self.auto_export_folder.trim();
        if folder.is_empty()
            || folder
                .split('/')
                .any(|part| part.trim().is_empty() || part.trim() == "." || part.trim() == "..")
            || folder.contains(['\\', ':', '*', '?', '"', '<', '>', '|'])
        {
            return Err(
                "the installers' folder must be a folder name such as \"For installers\" \
                 (use / for a subfolder)"
                    .into(),
            );
        }
        if self.offset_warn_m > self.offset_reject_m {
            return Err("offset_warn_m must not exceed offset_reject_m".into());
        }
        Ok(())
    }
}
