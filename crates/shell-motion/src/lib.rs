//! Spring physics for animation.
//!
//! A spring has no fixed duration, keeps its velocity when retargeted mid-flight, and
//! settles naturally, which is why design systems such as Material 3 move things with
//! springs rather than timed curves. `shell-m3e` has Material's spring specs.
//!
//! A [`Spring`] is advanced to a point in time with [`Spring::update`]; in `shell-ui`,
//! `Frame::animate` does that for the frame being built and keeps the window redrawing
//! every frame until the spring has settled.

use std::time::{Duration, Instant};

/// A damped spring's character. With unit mass, `stiffness` sets its speed and
/// `damping_ratio` its bounce: 1 settles without overshoot, lower values overshoot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringSpec {
    pub damping_ratio: f32,
    pub stiffness: f32,
}

impl SpringSpec {
    pub const fn new(damping_ratio: f32, stiffness: f32) -> Self {
        Self { damping_ratio, stiffness }
    }
}

/// Integration step. Small enough to be stable for the stiffest Material spring
/// (stiffness 3800).
const STEP: f32 = 1.0 / 1000.0;
/// A spring that hasn't been advanced for longer than this (its window was hidden, say)
/// resumes from where it was rather than jumping ahead.
const MAX_FRAME_GAP: Duration = Duration::from_millis(50);

/// An animated `f32`, pulled toward a target by a spring.
#[derive(Clone, Debug)]
pub struct Spring {
    spec: SpringSpec,
    value: f32,
    velocity: f32,
    target: f32,
    /// Considered settled within this distance of the target, at a speed this small.
    precision: f32,
    /// When it was last advanced; `None` while at rest.
    last: Option<Instant>,
}

impl Spring {
    /// A spring at rest at `value`.
    pub fn new(spec: SpringSpec, value: f32) -> Self {
        Self { spec, value, velocity: 0.0, target: value, precision: 1e-3, last: None }
    }

    /// How close to the target counts as arrived. Defaults to 0.001; for values measured
    /// in pixels, something like 0.01 settles sooner with no visible difference.
    pub fn with_precision(mut self, precision: f32) -> Self {
        self.precision = precision;
        self
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    /// Starts moving toward `target`, keeping the current velocity.
    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Jumps to `value` and stops.
    pub fn snap(&mut self, value: f32) {
        *self = Self { value, velocity: 0.0, target: value, last: None, ..*self };
    }

    pub fn is_moving(&self) -> bool {
        self.value != self.target || self.velocity != 0.0
    }

    /// Advances the simulation to `now` and returns the new value.
    pub fn update(&mut self, now: Instant) -> f32 {
        if !self.is_moving() {
            self.last = None;
            return self.value;
        }
        // The first frame after starting from rest only records the time.
        let elapsed = self.last.map_or(Duration::ZERO, |last| now.saturating_duration_since(last));
        self.last = Some(now);

        let mut remaining = elapsed.min(MAX_FRAME_GAP).as_secs_f32();
        let SpringSpec { damping_ratio, stiffness } = self.spec;
        let damping = 2.0 * damping_ratio * stiffness.sqrt();
        while remaining > 0.0 {
            // Semi-implicit Euler: stable for oscillators at this step size.
            let dt = remaining.min(STEP);
            let force = -stiffness * (self.value - self.target) - damping * self.velocity;
            self.velocity += force * dt;
            self.value += self.velocity * dt;
            remaining -= dt;
        }

        if (self.value - self.target).abs() < self.precision
            && self.velocity.abs() < self.precision * 10.0
        {
            self.value = self.target;
            self.velocity = 0.0;
        }
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNCY: SpringSpec = SpringSpec::new(0.6, 800.0);
    const SMOOTH: SpringSpec = SpringSpec::new(1.0, 1600.0);
    const SLOW: SpringSpec = SpringSpec::new(0.8, 380.0);

    /// Runs `spring` at 60 fps until it settles; returns (frames, peak value).
    fn run(spring: &mut Spring) -> (u32, f32) {
        let start = Instant::now();
        let (mut frames, mut peak) = (0, spring.value());
        while spring.is_moving() {
            frames += 1;
            assert!(frames < 600, "never settled");
            peak = peak.max(spring.update(start + Duration::from_secs_f32(frames as f32 / 60.0)));
        }
        (frames, peak)
    }

    #[test]
    fn underdamped_springs_overshoot_and_critically_damped_ones_dont() {
        let mut bouncy = Spring::new(BOUNCY, 0.0);
        bouncy.set_target(1.0);
        let (frames, peak) = run(&mut bouncy);
        assert_eq!(bouncy.value(), 1.0);
        assert!(peak > 1.05, "peak {peak}");
        assert!(frames < 60, "took {frames} frames");

        let mut smooth = Spring::new(SMOOTH, 0.0);
        smooth.set_target(1.0);
        let (_, peak) = run(&mut smooth);
        assert!(peak <= 1.0 + 1e-3, "peak {peak}");
    }

    #[test]
    fn a_long_pause_does_not_skip_ahead() {
        let start = Instant::now();
        let mut spring = Spring::new(SLOW, 0.0);
        spring.set_target(100.0);
        spring.update(start);
        let after_pause = spring.update(start + Duration::from_secs(10));
        assert!(after_pause < 50.0, "jumped to {after_pause}");
    }

    #[test]
    fn rest_is_not_moving() {
        let mut spring = Spring::new(SLOW, 3.0);
        assert!(!spring.is_moving());
        assert_eq!(spring.update(Instant::now()), 3.0);
        spring.set_target(3.0);
        assert!(!spring.is_moving());
    }
}
