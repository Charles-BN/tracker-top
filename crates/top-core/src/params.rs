//! Design parameters. Defaults reproduce `SLSQP_ToP.py`.

#[derive(Clone, Debug)]
pub struct Params {
    /// Nominal pile reveal above ground (m). Python: `Adapted Reveal`.
    pub reveal_nominal: f64,
    /// Lowest allowed reveal = nominal + `delta_min` (m). Python: -0.25 -> 1.20 m.
    pub delta_min: f64,
    /// Highest allowed reveal = nominal + `delta_max` (m). Python: +0.15 -> 1.60 m.
    pub delta_max: f64,
    /// Maximum angle change between two consecutive spans (degrees).
    /// The tracker limit is 0.75 deg; the Python script uses 0.70 deg as margin.
    pub max_angle_deg: f64,
    /// Slack when judging compliance of the final result (degrees).
    pub angle_tolerance_deg: f64,
    /// Weight on squared cut/fill in the objective. Large = grade only when forced.
    pub cut_fill_penalty: f64,
    /// Hard cap on cut/fill at any pile (m).
    pub max_cut_fill: f64,
    /// Cut/fill above this (m) counts the tracker as "Graded" rather than "Compliant".
    pub grading_threshold: f64,
    /// A pile is flagged as a LIDAR outlier if |z - mean| > alpha * std of its tracker.
    pub lidar_sigma_alpha: f64,
    /// Keep each torque-tube span at its design length: piles are set out along
    /// the tracker from the motor pile (see [`crate::setout`]).
    pub fixed_spans: bool,
    /// Round tops, cut/fill and set-out positions to this step (m), e.g. 0.01
    /// for the centimetre; 0 keeps the exact values. The limits are checked on
    /// the rounded values (see [`crate::optimize`]).
    pub round_step: f64,
    /// Weight of the pull towards the neighbouring rows (see [`crate::align`]),
    /// relative to keeping the nominal reveal; 0 = off.
    pub align_weight: f64,
    /// Steepest slope of any one span (deg, e.g. ATI's LTTS < 15°); 0 = no limit.
    pub max_span_slope_deg: f64,
    /// Steepest slope of the whole tracker, first pile to last (deg, e.g.
    /// ATI's ATTS < 8.5°); 0 = no limit.
    pub max_tracker_slope_deg: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            reveal_nominal: 1.45,
            delta_min: -0.25,
            delta_max: 0.15,
            max_angle_deg: 0.70,
            angle_tolerance_deg: 0.001,
            cut_fill_penalty: 1.0e3,
            max_cut_fill: 1.0,
            grading_threshold: 0.005,
            lidar_sigma_alpha: 2.0,
            fixed_spans: false,
            round_step: 0.0,
            align_weight: 0.0,
            max_span_slope_deg: 0.0,
            max_tracker_slope_deg: 0.0,
        }
    }
}

impl Params {
    pub fn reveal_min(&self) -> f64 {
        self.reveal_nominal + self.delta_min
    }

    pub fn reveal_max(&self) -> f64 {
        self.reveal_nominal + self.delta_max
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(self.delta_min < self.delta_max) {
            return Err("delta_min must be smaller than delta_max".into());
        }
        if !(self.max_angle_deg > 0.0 && self.max_angle_deg < 45.0) {
            return Err("max_angle_deg must be in (0, 45)".into());
        }
        if !(self.cut_fill_penalty > 0.0) {
            return Err("cut_fill_penalty must be positive".into());
        }
        if !(self.max_cut_fill >= 0.0) {
            return Err("max_cut_fill must be >= 0".into());
        }
        for (name, v) in [
            ("max_span_slope_deg", self.max_span_slope_deg),
            ("max_tracker_slope_deg", self.max_tracker_slope_deg),
        ] {
            if !(v.is_finite() && (0.0..60.0).contains(&v)) {
                return Err(format!("{name} must be between 0 (no limit) and 60 degrees"));
            }
        }
        if !(self.align_weight >= 0.0 && self.align_weight.is_finite()) {
            return Err("align_weight must be >= 0".into());
        }
        if !(self.round_step >= 0.0 && self.round_step <= 0.05) {
            return Err("round_step must be between 0 and 0.05 m".into());
        }
        Ok(())
    }
}
