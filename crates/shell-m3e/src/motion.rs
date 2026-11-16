//! Material 3's motion schemes, as specs for [`shell_motion::Spring`]s.
//!
//! Material 3 pairs "spatial" springs, for anything that changes position, size or
//! shape (and may overshoot), with "effects" springs, for color and opacity (which never
//! overshoot).

pub use shell_motion::{Spring, SpringSpec};

/// Material 3's expressive motion scheme, "for prominent UI elements and hero
/// interactions" (values from androidx's `ExpressiveMotionTokens`).
pub mod expressive {
    use super::SpringSpec;

    pub const SPATIAL_FAST: SpringSpec = SpringSpec::new(0.6, 800.0);
    pub const SPATIAL: SpringSpec = SpringSpec::new(0.8, 380.0);
    pub const SPATIAL_SLOW: SpringSpec = SpringSpec::new(0.8, 200.0);
    pub const EFFECTS_FAST: SpringSpec = SpringSpec::new(1.0, 3800.0);
    pub const EFFECTS: SpringSpec = SpringSpec::new(1.0, 1600.0);
    pub const EFFECTS_SLOW: SpringSpec = SpringSpec::new(1.0, 800.0);
}

/// Material 3's standard motion scheme, "for utilitarian UI elements and recurring
/// interactions" (values from androidx's `StandardMotionTokens`).
pub mod standard {
    use super::SpringSpec;

    pub const SPATIAL_FAST: SpringSpec = SpringSpec::new(0.9, 1400.0);
    pub const SPATIAL: SpringSpec = SpringSpec::new(0.9, 700.0);
    pub const SPATIAL_SLOW: SpringSpec = SpringSpec::new(0.9, 300.0);
    pub const EFFECTS_FAST: SpringSpec = SpringSpec::new(1.0, 3800.0);
    pub const EFFECTS: SpringSpec = SpringSpec::new(1.0, 1600.0);
    pub const EFFECTS_SLOW: SpringSpec = SpringSpec::new(1.0, 800.0);
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// Runs `spring` toward 1 at 60 fps until it settles; returns its peak.
    fn peak(spec: SpringSpec) -> f32 {
        let start = Instant::now();
        let mut spring = Spring::new(spec, 0.0);
        spring.set_target(1.0);
        let mut peak: f32 = 0.0;
        for frame in 1..600 {
            peak = peak.max(spring.update(start + Duration::from_secs_f32(frame as f32 / 60.0)));
            if !spring.is_moving() {
                return peak;
            }
        }
        panic!("never settled");
    }

    #[test]
    fn spatial_springs_overshoot_and_effects_springs_dont() {
        for spec in [expressive::SPATIAL_FAST, expressive::SPATIAL, expressive::SPATIAL_SLOW] {
            assert!(peak(spec) > 1.01, "{spec:?}");
        }
        for spec in [expressive::EFFECTS_FAST, expressive::EFFECTS, standard::EFFECTS_SLOW] {
            assert!(peak(spec) <= 1.0 + 1e-3, "{spec:?}");
        }
    }
}
