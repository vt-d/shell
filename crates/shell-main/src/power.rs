//! The power button: a scalloped Material shape at the bar's end, the power icon in it.
//!
//! It is the clock's escapement. Each time the minute turns it clicks round by one
//! scallop, on Material's expressive spatial spring, so it overshoots a touch and
//! settles looking just as it did. Clicking it opens the power menu.

use std::f32::consts::TAU;

use shell_m3e::{
    Icon, MaterialShape, Theme, ThemeExt,
    components::Touch,
    motion::{Spring, expressive},
};
use shell_services::quick::Switch;
use shell_ui::{AlignItems, Element, Frame, PointerEvent, PointerKind, Shape};

use crate::{
    quick::{label, state_layer, tile_glass},
    status::HEIGHT,
};

const BUTTON: u32 = 22;
const FIRST_ACTION: u32 = 80;

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
    touch: Touch,
}

impl Power {
    pub fn new() -> Self {
        Self { turn: Spring::new(expressive::SPATIAL, 0.0), touch: Touch::new() }
    }

    /// Clicks round by one scallop.
    pub fn tick(&mut self) {
        self.turn.set_target(self.turn.target() + 1.0);
    }

    pub fn pointer_event(&mut self, event: PointerEvent) -> bool {
        if event.target != Some(BUTTON) {
            return false;
        }
        self.touch.handle(event.kind);
        event.kind == PointerKind::Click
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
            .target(BUTTON)
            .background(
                theme.primary.lerp(theme.on_primary, state_layer(self.touch.animate(frame))),
            )
            .shape(Shape::new(SHAPE).rotated(turn * TAU / SCALLOPS).scaled(SCALE))
            .child(Element::symbol(icon).text_color(theme.on_primary))
    }
}

struct Entry {
    icon: &'static str,
    label: &'static str,
    confirmation: Option<&'static str>,
    switch: Switch,
}

const ENTRIES: [Entry; 3] = [
    Entry { icon: "bedtime", label: "Suspend", confirmation: None, switch: Switch::Suspend },
    Entry {
        icon: "restart_alt",
        label: "Restart",
        confirmation: Some("Confirm restart"),
        switch: Switch::Restart,
    },
    Entry {
        icon: "power_settings_new",
        label: "Power off",
        confirmation: Some("Confirm power off"),
        switch: Switch::PowerOff,
    },
];

#[derive(Default)]
pub struct Menu {
    touches: [Touch; ENTRIES.len()],
    armed: Option<usize>,
}

impl Menu {
    pub const WIDTH: f32 = 236.0;
    const PADDING: f32 = 12.0;
    const ROW: f32 = 48.0;
    const GAP: f32 = 4.0;
    pub const HEIGHT: f32 = 2.0 * Self::PADDING
        + ENTRIES.len() as f32 * Self::ROW
        + (ENTRIES.len() - 1) as f32 * Self::GAP;

    pub fn view(&mut self, frame: &mut Frame, theme: &Theme) -> Element {
        let mut menu = Element::column().width(Self::WIDTH).padding(Self::PADDING).gap(Self::GAP);
        for (index, entry) in ENTRIES.iter().enumerate() {
            let confirmation = entry.confirmation.filter(|_| self.armed == Some(index));
            let (fill, ink) = if confirmation.is_some() {
                (theme.error_container.with_alpha(0.8), theme.on_error_container)
            } else {
                (tile_glass(theme), theme.on_surface)
            };
            let touched = self.touches[index].animate(frame);
            let top = if index == 0 { 16.0 } else { 6.0 };
            let bottom = if index + 1 == ENTRIES.len() { 16.0 } else { 6.0 };
            menu = menu.child(
                Element::row()
                    .height(Self::ROW)
                    .align_items(AlignItems::CENTER)
                    .gap(14.0)
                    .padding_xy(16.0, 0.0)
                    .corner_radii(top, top, bottom, bottom)
                    .target(FIRST_ACTION + index as u32)
                    .background(fill.lerp(ink, state_layer(touched)))
                    .child(Element::symbol(Icon::new(entry.icon).size(22.0)).text_color(ink))
                    .child(label(confirmation.unwrap_or(entry.label), 16.0, 600, ink)),
            );
        }
        menu
    }

    pub fn event(&mut self, id: u32, kind: PointerKind) -> Option<Switch> {
        let index = id.checked_sub(FIRST_ACTION)? as usize;
        let entry = ENTRIES.get(index)?;
        self.touches[index].handle(kind);
        match kind {
            PointerKind::Click if entry.confirmation.is_none() || self.armed == Some(index) => {
                self.armed = None;
                Some(entry.switch)
            }
            PointerKind::Click => {
                self.armed = Some(index);
                None
            }
            PointerKind::Leave if self.armed == Some(index) => {
                self.armed = None;
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_in_the_font() {
        assert!(shell_m3e::icons::font().contains("power_settings_new"));
    }

    #[test]
    fn bar_button_opens_the_menu_only_on_click() {
        let mut power = Power::new();
        let event = PointerEvent {
            target: Some(BUTTON),
            kind: PointerKind::Press,
            x: 0.0,
            y: 0.0,
            width: HEIGHT,
            height: HEIGHT,
        };
        assert!(!power.pointer_event(event));
        assert!(power.pointer_event(PointerEvent { kind: PointerKind::Click, ..event }));
        assert!(!power.pointer_event(PointerEvent {
            target: None,
            kind: PointerKind::Click,
            ..event
        }));
    }

    #[test]
    fn power_choices_dispatch_the_selected_action_and_leaving_cancels_confirmation() {
        let mut menu = Menu::default();
        assert_eq!(menu.event(FIRST_ACTION, PointerKind::Click), Some(Switch::Suspend));
        for (id, action) in
            [(FIRST_ACTION + 1, Switch::Restart), (FIRST_ACTION + 2, Switch::PowerOff)]
        {
            assert_eq!(menu.event(id, PointerKind::Click), None);
            assert_eq!(menu.event(id, PointerKind::Leave), None);
            assert_eq!(menu.event(id, PointerKind::Click), None);
            assert_eq!(menu.event(id, PointerKind::Click), Some(action));
        }
    }
}
