//! The clock: the time alone, as the last segment of the [status](crate::status) group.
//!
//! It is set plainly: one weight, one color, the typeface's own spacing and colon. The
//! time is the bar's one line of emphasized type, in `primary` on the group's quiet fill,
//! so it stands out by being type rather than by being a different color of segment.
//!
//! When the minute turns, the minutes roll: the old pair leaves upward as the new one
//! springs up from below, landing with a little overshoot. On the hour, the hours roll
//! first and the minutes follow.

use shell_m3e::{
    ThemeExt,
    motion::{Spring, expressive},
};
use shell_services::clock::Time;
use shell_ui::{AlignItems, Color, Element, Frame};

use crate::status::{HEIGHT, INNER_RADIUS, pill_fill};

/// Title large's size, emphasized.
const SIZE: f32 = 22.0;
const WEIGHT: u16 = 800;
/// Space between the time and the group's end, a Small button's; and between it and the
/// seam with the segment before.
const PADDING: f32 = 16.0;
const SEAM_PADDING: f32 = 12.0;
/// Space either side of the colon.
const COLON_GAP: f32 = 1.0;
/// How strongly the colon shows beside the digits.
const COLON_OPACITY: f32 = 0.6;
/// How far a pair of digits travels as it rolls in or out.
const TRAVEL: f32 = 12.0;
/// How much later the minutes start rolling when the hours roll too, as a fraction of
/// the roll.
const STAGGER: f32 = 0.35;

pub struct Clock {
    hours: Pair,
    minutes: Pair,
}

/// Two digits that roll together.
struct Pair {
    current: u8,
    /// The value rolling out, if it just changed.
    previous: Option<u8>,
    /// 0 → 1 as `previous` gives way to `current`; below 0 while waiting its turn.
    roll: Spring,
}

impl Pair {
    fn new(value: u8) -> Self {
        Self { current: value, previous: None, roll: Spring::new(expressive::SPATIAL, 1.0) }
    }

    /// Rolls to `value`, after `wait` of a roll.
    fn set(&mut self, value: u8, wait: f32) {
        self.previous = Some(self.current);
        self.current = value;
        self.roll.snap(-wait);
        self.roll.set_target(1.0);
    }

    fn view(&mut self, frame: &mut Frame, color: Color) -> Element {
        let roll = frame.animate(&mut self.roll);
        if !self.roll.is_moving() {
            self.previous = None;
        }
        let face = |value: u8, opacity: f32| {
            Element::text(format!("{value:02}"))
                .wrap(false)
                .height(HEIGHT)
                .font_size(SIZE)
                .font_weight(WEIGHT)
                .text_color(color.with_alpha(color.a * opacity))
        };
        let slot = Element::row().height(HEIGHT).shrink(0.0);
        let Some(previous) = self.previous else {
            return slot.child(face(self.current, 1.0));
        };
        // Position follows the spring, overshoot and all; opacity stays within range.
        let position = roll.max(0.0);
        let shown = roll.clamp(0.0, 1.0);
        // The new pair is in the flow, so the slot is as wide as it; the old one is gone
        // well before the new one lands.
        slot.child(
            face(previous, 1.0 - (shown * 1.5).min(1.0))
                .absolute(0.0, 0.0)
                .offset(0.0, -TRAVEL * position),
        )
        .child(face(self.current, shown).offset(0.0, TRAVEL * (1.0 - position)))
    }
}

impl Clock {
    pub fn new(time: Time) -> Self {
        Self { hours: Pair::new(time.hour), minutes: Pair::new(time.minute) }
    }

    pub fn set_time(&mut self, time: Time) {
        let mut wait = 0.0;
        if self.hours.current != time.hour {
            self.hours.set(time.hour, wait);
            wait += STAGGER;
        }
        if self.minutes.current != time.minute {
            self.minutes.set(time.minute, wait);
        }
    }

    pub fn view(&mut self, frame: &mut Frame) -> Element {
        let theme = frame.theme();
        let color = theme.primary;
        Element::row()
            .align_items(AlignItems::CENTER)
            .height(HEIGHT)
            .shrink(0.0)
            .padding_sides(0.0, PADDING, 0.0, SEAM_PADDING)
            .gap(COLON_GAP)
            .background(pill_fill(theme))
            .corner_radii(INNER_RADIUS, HEIGHT / 2.0, HEIGHT / 2.0, INNER_RADIUS)
            .child(self.hours.view(frame, color))
            .child(
                Element::text(":")
                    .wrap(false)
                    .height(HEIGHT)
                    .font_size(SIZE)
                    .font_weight(WEIGHT)
                    .text_color(color.with_alpha(color.a * COLON_OPACITY)),
            )
            .child(self.minutes.view(frame, color))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u8, minute: u8) -> Time {
        Time { hour, minute, ..Time::default() }
    }

    #[test]
    fn only_what_changes_rolls_and_the_minutes_follow_the_hours() {
        let mut clock = Clock::new(at(9, 59));
        clock.set_time(at(10, 0));
        assert_eq!(clock.hours.previous, Some(9));
        assert_eq!(clock.minutes.previous, Some(59));
        assert!(clock.minutes.roll.value() < clock.hours.roll.value());

        let mut clock = Clock::new(at(10, 0));
        clock.set_time(at(10, 1));
        assert_eq!(clock.hours.previous, None);
        assert_eq!(clock.minutes.previous, Some(0));
        assert_eq!(clock.minutes.roll.value(), 0.0);
    }
}
