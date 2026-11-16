//! The power button: a scalloped Material shape at the bar's end, the power icon in it.
//!
//! It is the clock's escapement. Each time the minute turns it clicks round by one
//! scallop, on Material's expressive spatial spring, so it overshoots a touch and
//! settles looking just as it did. (Display only until the UI takes pointer input.)

use std::f32::consts::TAU;

use shell_m3e::{
    Icon, MaterialShape, ThemeExt,
    motion::{Spring, expressive},
};
use shell_ui::{Element, Frame, Shape};

use crate::status::HEIGHT;

const SHAPE: MaterialShape = MaterialShape::Cookie9Sided;
/// How many scallops `SHAPE` has: it looks the same after a turn of one.
const SCALLOPS: f32 = 9.0;
/// The shape is drawn a little larger than the pills are tall: scallops cut into it, so
/// at their height it would look smaller than they do.
const SCALE: f32 = 1.1;
const ICON: f32 = 20.0;
/// Heavier than the status icons: it stands alone on a strong color.
const ICON_WEIGHT: f32 = 600.0;

pub struct Power {
    /// How many scallops round it has turned.
    turn: Spring,
}

impl Power {
    pub fn new() -> Self {
        Self { turn: Spring::new(expressive::SPATIAL, 0.0) }
    }

    /// Clicks round by one scallop.
    pub fn tick(&mut self) {
        self.turn.set_target(self.turn.target() + 1.0);
    }

    pub fn view(&mut self, frame: &mut Frame) -> Element {
        let theme = frame.theme();
        let turn = frame.animate(&mut self.turn);
        if !self.turn.is_moving() && turn >= SCALLOPS {
            // A whole turn on: the same angle.
            self.turn.snap(turn - SCALLOPS);
        }
        let icon = Icon::new("power_settings_new").size(ICON).weight(ICON_WEIGHT);
        Element::row()
            .center()
            .size(HEIGHT, HEIGHT)
            .shrink(0.0)
            .background(theme.primary)
            .shape(Shape::new(SHAPE).rotated(turn * TAU / SCALLOPS).scaled(SCALE))
            .child(Element::symbol(icon).text_color(theme.on_primary))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_icon_is_in_the_font() {
        assert!(shell_m3e::icons::font().contains("power_settings_new"));
    }
}
