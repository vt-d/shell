//! Material 3 Expressive controls: what takes pointer input.
//!
//! - [`Touch`]: a control's hover and press, animated; the state every control is drawn
//!   from.
//! - [`button`]: a container that shows a state layer under the pointer and squares its
//!   corners off when pressed, as Expressive buttons do. Toggle buttons also change shape
//!   when selected.
//! - [`slider`]: Expressive's slider: a thick track split by a handle that narrows when
//!   held.
//!
//! Controls are plain functions from state to [`Element`]s. Each takes the `id` its
//! pointer events are [reported with](Element::target); route those events' kinds to its
//! [`Touch`] and act on the clicks and drags.

use shell_ui::{Color, Element, Frame, PointerEvent, PointerKind};

use crate::{
    Theme,
    motion::{Spring, expressive},
};

/// A state layer's opacity under the pointer, and when pressed (on top of that).
const HOVER_OPACITY: f32 = 0.08;
const PRESS_OPACITY: f32 = 0.10;

/// A control's hover and press, animated.
pub struct Touch {
    hover: Spring,
    press: Spring,
}

/// How far a [`Touch`] is hovered and pressed, each 0–1 (a press may overshoot a touch).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Touched {
    pub hover: f32,
    pub press: f32,
}

impl Touched {
    /// The state layer's opacity.
    fn layer(self) -> f32 {
        HOVER_OPACITY * self.hover.clamp(0.0, 1.0) + PRESS_OPACITY * self.press.clamp(0.0, 1.0)
    }
}

impl Default for Touch {
    fn default() -> Self {
        Self::new()
    }
}

impl Touch {
    pub fn new() -> Self {
        Self {
            hover: Spring::new(expressive::EFFECTS_FAST, 0.0),
            press: Spring::new(expressive::SPATIAL_FAST, 0.0),
        }
    }

    /// Takes in what the pointer did to the control.
    pub fn handle(&mut self, kind: PointerKind) {
        match kind {
            PointerKind::Enter => self.hover.set_target(1.0),
            PointerKind::Leave => self.hover.set_target(0.0),
            PointerKind::Press => self.press.set_target(1.0),
            PointerKind::Release => self.press.set_target(0.0),
            PointerKind::Drag
            | PointerKind::Click
            | PointerKind::SecondaryPress
            | PointerKind::MiddlePress
            | PointerKind::Scroll(_) => {}
        }
    }

    /// Whether the button is down on it.
    pub fn held(&self) -> bool {
        self.press.target() > 0.0
    }

    pub fn animate(&mut self, frame: &mut Frame) -> Touched {
        Touched { hover: frame.animate(&mut self.hover), press: frame.animate(&mut self.press) }
    }
}

/// How a [`button`] looks.
#[derive(Clone, Copy, Debug)]
pub struct Button {
    pub width: f32,
    pub height: f32,
    pub fill: Color,
    /// The color of what's on it, which its state layer is a veil of.
    pub content: Color,
    /// 0 is fully rounded; 1 squared off, as a selected toggle button is. Animate it.
    pub squared: f32,
}

impl Button {
    /// Its corner radius when squared off and when pressed, as fractions of its height:
    /// Material's medium and small corners on a 40dp button.
    const SQUARED: f32 = 0.3;
    const PRESSED: f32 = 0.2;

    fn radius(&self, touched: Touched) -> f32 {
        let round = self.height / 2.0;
        let rest = round + (self.height * Self::SQUARED - round) * self.squared;
        rest + (self.height * Self::PRESSED - rest) * touched.press
    }
}

/// A button showing `content`, centered. Its events have `id` as their target.
pub fn button(id: u32, button: Button, touched: Touched, content: Element) -> Element {
    let radius = button.radius(touched).max(0.0);
    Element::row()
        .center()
        .size(button.width, button.height)
        .shrink(0.0)
        .background(button.fill)
        .corner_radius(radius)
        .target(id)
        .child(
            Element::new()
                .absolute(0.0, 0.0)
                .size(button.width, button.height)
                .background(button.content.with_alpha(button.content.a * touched.layer()))
                .corner_radius(radius),
        )
        .child(content)
}

/// How a [`slider`] looks.
#[derive(Clone, Copy, Debug)]
pub struct Slider {
    pub width: f32,
    /// The whole control's height, which is the handle's.
    pub height: f32,
    pub track: f32,
    pub active: Color,
    pub inactive: Color,
}

impl Slider {
    /// The handle's width at rest and held, and the space either side of it.
    const HANDLE: f32 = 4.0;
    const HANDLE_HELD: f32 = 2.0;
    const GAP: f32 = 6.0;
    /// The tracks' corners where they face the handle.
    const INNER_RADIUS: f32 = 2.0;
    /// The dot at the inactive track's end.
    const STOP: f32 = 4.0;

    /// Material's extra-small slider, in the theme's colors.
    pub fn new(theme: &Theme, width: f32) -> Self {
        Self {
            width,
            height: 36.0,
            track: 16.0,
            active: theme.primary,
            inactive: theme.secondary_container,
        }
    }

    /// The value (0–1) the pointer is at in `event`, a press or drag on the slider.
    pub fn value_at(event: &PointerEvent) -> f32 {
        let inset = Self::HANDLE / 2.0;
        ((event.x - inset) / (event.width - 2.0 * inset).max(1.0)).clamp(0.0, 1.0)
    }
}

/// A slider showing `value` (0–1). Its events have `id` as their target; see
/// [`Slider::value_at`].
pub fn slider(id: u32, slider: Slider, value: f32, touched: Touched) -> Element {
    let held = touched.press.clamp(0.0, 1.0);
    let handle = Slider::HANDLE + (Slider::HANDLE_HELD - Slider::HANDLE) * held;
    let center = Slider::HANDLE / 2.0 + (slider.width - Slider::HANDLE) * value.clamp(0.0, 1.0);
    let top = (slider.height - slider.track) / 2.0;
    let outer = slider.track / 2.0;
    let active = (center - handle / 2.0 - Slider::GAP).max(0.0);
    let rest = (center + handle / 2.0 + Slider::GAP).min(slider.width);

    let mut element = Element::new()
        .size(slider.width, slider.height)
        .shrink(0.0)
        .target(id)
        .child(
            Element::new()
                .absolute(0.0, top)
                .size(active, slider.track)
                .background(slider.active)
                .corner_radii(outer, Slider::INNER_RADIUS, Slider::INNER_RADIUS, outer),
        )
        .child(
            Element::new()
                .absolute(rest, top)
                .size(slider.width - rest, slider.track)
                .background(slider.inactive)
                .corner_radii(Slider::INNER_RADIUS, outer, outer, Slider::INNER_RADIUS),
        );
    // The stop, fading out as the handle closes in on it.
    let clear = ((slider.width - rest - slider.track / 2.0) / slider.track).clamp(0.0, 1.0);
    if clear > 0.0 {
        let inset = (slider.track - Slider::STOP) / 2.0;
        element = element.child(
            Element::new()
                .absolute(slider.width - inset - Slider::STOP, top + inset)
                .size(Slider::STOP, Slider::STOP)
                .background(slider.active.with_alpha(slider.active.a * clear))
                .corner_radius(Slider::STOP / 2.0),
        );
    }
    element.child(
        Element::new()
            .absolute(center - handle / 2.0, 0.0)
            .size(handle, slider.height)
            .background(slider.active)
            .corner_radius(handle / 2.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slider_reads_its_ends_and_middle_from_the_pointer() {
        let at = |x: f32| {
            Slider::value_at(&PointerEvent {
                target: Some(0),
                kind: PointerKind::Drag,
                x,
                y: 0.0,
                width: 204.0,
                height: 36.0,
            })
        };
        assert_eq!(at(-30.0), 0.0);
        assert_eq!(at(102.0), 0.5);
        assert_eq!(at(500.0), 1.0);
    }

    #[test]
    fn buttons_square_off_when_selected_and_more_when_pressed() {
        let button = |squared| Button {
            width: 40.0,
            height: 40.0,
            fill: Color::BLACK,
            content: Color::WHITE,
            squared,
        };
        let rest = Touched::default();
        let pressed = Touched { hover: 1.0, press: 1.0 };
        assert_eq!(button(0.0).radius(rest), 20.0);
        assert_eq!(button(1.0).radius(rest), 12.0);
        assert_eq!(button(0.0).radius(pressed), 8.0);
    }
}
