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

/// Where the tracker's heights are given (the line the optimiser works on).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Technology {
    /// Every pile ends at the tube: heights are the tops of pile and the
    /// window is the reveal (Nextracker NX Horizon / XTR).
    #[default]
    Top,
    /// Heights are the torque tube's; each pile type sits a fixed distance
    /// below it (`PileRule::drop_m`) and the window is the tube height above
    /// the ground, TTH (ATI OmniTrack).
    Tube,
}

/// What a pile type is (matched on the master list's Pile Type, ignoring case).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PileRule {
    pub pile_type: String,
    /// How far the top of this pile sits below the torque tube (m); used
    /// with `Technology::Tube` only.
    pub drop_m: f64,
    /// It carries the drive: the set-out keeps it fixed.
    pub motor: bool,
}

impl Default for PileRule {
    fn default() -> Self {
        Self {
            pile_type: String::new(),
            drop_m: 0.0,
            motor: false,
        }
    }
}

/// A pile part's (or type's) own lowest and highest reveal: the top of pile
/// above the ground, e.g. a clearance under the modules, or the pile length
/// minus its minimum embedment. Matched on the master list's Description,
/// else its Pile Type (ignoring case).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PileLimit {
    pub key: String,
    pub min_reveal_m: Option<f64>,
    pub max_reveal_m: Option<f64>,
}

/// Weight of the pull towards the neighbouring rows when `align_rows` is on,
/// relative to keeping the nominal reveal (see `top_core::align`).
pub const ALIGN_WEIGHT: f64 = 10.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where heights are given: tops of pile, or torque tube.
    pub technology: Technology,
    /// Pile types with their drop below the tube and their role. A type
    /// without a rule is a motor pile when its name says so (`HMP`, "Motor").
    pub pile_rules: Vec<PileRule>,
    /// Lowest / highest reveal of pile parts or types (see `PileLimit`).
    pub pile_limits: Vec<PileLimit>,
    /// Steepest slope of one span of torque tube (deg, ATI's LTTS); 0 = none.
    pub max_span_slope_deg: f64,
    /// Steepest slope of a whole tracker, first pile to last (deg, ATI's
    /// ATTS); 0 = none.
    pub max_tracker_slope_deg: f64,
    /// Nominal reveal above ground (m); with `Technology::Tube`, the nominal
    /// torque-tube height (TTH). Rows with their own design height in the
    /// master list (`pile_design.nominal`) use theirs.
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
    /// Pull each row's tops towards the line through its neighbouring rows
    /// (less shade between rows when the trackers backtrack); blocks ticked
    /// as issued to the installers keep their tops.
    pub align_rows: bool,
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
            technology: Technology::Top,
            pile_rules: Vec::new(),
            pile_limits: Vec::new(),
            max_span_slope_deg: 0.0,
            max_tracker_slope_deg: 0.0,
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
            align_rows: false,
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
            align_weight: if self.align_rows { ALIGN_WEIGHT } else { 0.0 },
            max_span_slope_deg: self.max_span_slope_deg,
            max_tracker_slope_deg: self.max_tracker_slope_deg,
            ..Params::default()
        }
    }

    /// The parameters that change optimisation results, as stable JSON. Part of
    /// every tracker's input signature, so editing them re-runs every tracker.
    /// (Row alignment is not in it: it is marked on each result instead, so
    /// that blocks issued to the installers keep theirs; see `Store::optimize`.)
    pub fn params_json(&self) -> String {
        let mut v = serde_json::json!({
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
        });
        // Only when used, so that results made before these existed still stand.
        if self.technology != Technology::Top || !self.pile_rules.is_empty() {
            v["technology"] = serde_json::json!(self.technology);
            v["pile_rules"] = serde_json::json!(self.pile_rules);
        }
        if !self.pile_limits.is_empty() {
            v["pile_limits"] = serde_json::json!(self.pile_limits);
        }
        if self.max_span_slope_deg > 0.0 || self.max_tracker_slope_deg > 0.0 {
            v["max_span_slope_deg"] = serde_json::json!(self.max_span_slope_deg);
            v["max_tracker_slope_deg"] = serde_json::json!(self.max_tracker_slope_deg);
        }
        v.to_string()
    }

    /// The rule for a pile type, if any (ignoring case and spaces at the ends).
    pub fn pile_rule(&self, pile_type: &str) -> Option<&PileRule> {
        let t = pile_type.trim();
        self.pile_rules
            .iter()
            .find(|r| r.pile_type.trim().eq_ignore_ascii_case(t))
    }

    /// The limit for a pile: its Description's, else its Pile Type's.
    pub fn pile_limit(&self, description: &str, pile_type: &str) -> Option<&PileLimit> {
        let find = |k: &str| {
            let k = k.trim();
            (!k.is_empty())
                .then(|| self.pile_limits.iter().find(|l| l.key.trim().eq_ignore_ascii_case(k)))
                .flatten()
        };
        find(description).or_else(|| find(pile_type))
    }

    /// Lowest and highest height of the line over the ground a pile's own
    /// limits allow (its reveal limits plus its drop), if it has any.
    pub fn pile_heights(&self, description: &str, pile_type: &str) -> (Option<f64>, Option<f64>) {
        let drop = self.pile_drop(pile_type);
        match self.pile_limit(description, pile_type) {
            Some(l) => (
                l.min_reveal_m.filter(|v| v.is_finite()).map(|v| v + drop),
                l.max_reveal_m.filter(|v| v.is_finite()).map(|v| v + drop),
            ),
            None => (None, None),
        }
    }

    /// The window of the line over the ground at a pile: around its design
    /// height, narrowed by its own limits.
    pub fn pile_window(&self, nominal: Option<f64>, description: &str, pile_type: &str) -> (f64, f64) {
        let nom = nominal.unwrap_or(self.reveal_nominal);
        let (lo, hi) = self.pile_heights(description, pile_type);
        (
            (nom + self.delta_min).max(lo.unwrap_or(f64::NEG_INFINITY)),
            (nom + self.delta_max).min(hi.unwrap_or(f64::INFINITY)),
        )
    }

    /// How far a pile of this type sits below the line the optimiser works on (m).
    pub fn pile_drop(&self, pile_type: &str) -> f64 {
        match self.technology {
            Technology::Top => 0.0,
            Technology::Tube => self.pile_rule(pile_type).map_or(0.0, |r| r.drop_m),
        }
    }

    /// Whether a pile of this type carries the drive.
    pub fn is_motor(&self, pile_type: &str) -> bool {
        match self.pile_rule(pile_type) {
            Some(r) => r.motor,
            None => top_core::setout::is_motor_pile(pile_type),
        }
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
        for l in &self.pile_limits {
            if l.key.trim().is_empty() {
                return Err("every pile limit needs a description or pile type".into());
            }
            for v in [l.min_reveal_m, l.max_reveal_m].into_iter().flatten() {
                if !(v.is_finite() && (0.0..=10.0).contains(&v)) {
                    return Err(format!("the reveal limits of \"{}\" must be between 0 and 10 m", l.key));
                }
            }
            if let (Some(a), Some(b)) = (l.min_reveal_m, l.max_reveal_m)
                && a > b
            {
                return Err(format!("\"{}\": the lowest reveal is above the highest", l.key));
            }
        }
        for r in &self.pile_rules {
            if r.pile_type.trim().is_empty() {
                return Err("every pile type rule needs a pile type".into());
            }
            if !(r.drop_m.is_finite() && (0.0..=5.0).contains(&r.drop_m)) {
                return Err(format!(
                    "the drop below the tube of pile type \"{}\" must be between 0 and 5 m",
                    r.pile_type
                ));
            }
        }
        if self.offset_warn_m > self.offset_reject_m {
            return Err("offset_warn_m must not exceed offset_reject_m".into());
        }
        Ok(())
    }
}
