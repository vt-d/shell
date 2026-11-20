//! Bluetooth detail inside the quick settings sheet.

use super::{
    ACCENT_GLASS, HEADER, INNER, PADDING, Scaled, TILE_GLASS, TOP_PADDING, WIDTH, label,
    state_layer, target, tile_glass,
};
use shell_m3e::{
    Icon, Theme,
    components::{Button, Touch, button},
    motion::{Spring, expressive},
};
use shell_services::quick::State;
use shell_ui::{AlignItems, Color, Element, Frame, PointerKind};

const ROW: f32 = 52.0;
const GAP: f32 = 2.0;
const LIST_HEIGHT: f32 = ROW * 2.0 + GAP;
const HEADER_GAP: f32 = 12.0;
const POWER_HEIGHT: f32 = 72.0;

#[derive(Default)]
pub(super) struct Menu {
    pub(super) back: Touch,
    pub(super) power: Touch,
    pub(super) loading: bool,
    switch: Option<Spring>,
    scroll_y: f32,
}

impl Menu {
    pub(super) fn scroll(&mut self, kind: PointerKind, state: &State) {
        if let PointerKind::Scroll(steps) = kind {
            let count = state.connected_devices.as_ref().map_or(0, Vec::len);
            self.scroll_y = (self.scroll_y + steps * (ROW + GAP)).clamp(0.0, max_scroll(count));
        }
    }

    pub(super) fn view(&mut self, frame: &mut Frame, theme: &Theme, state: &State) -> Element {
        let back = button(
            target::BACK,
            Button {
                width: HEADER,
                height: HEADER,
                fill: tile_glass(theme),
                content: theme.on_surface,
                squared: 0.0,
            },
            self.back.animate(frame),
            glyph("arrow_back", theme.on_surface),
        );
        let header = Element::row()
            .height(HEADER)
            .align_items(AlignItems::CENTER)
            .gap(16.0)
            .child(back)
            .child(label("Bluetooth", 28.0, 600, theme.on_surface));

        let mut page = Element::column()
            .width(WIDTH)
            .padding_sides(TOP_PADDING, PADDING, PADDING, PADDING)
            .child(header)
            .child(Element::new().height(HEADER_GAP))
            .child(self.power_row(frame, theme, state.bluetooth));
        if state.bluetooth == Some(true) {
            page = page
                .child(Element::new().height(16.0))
                .child(label("Connected devices", 14.0, 600, theme.on_surface_variant))
                .child(Element::new().height(8.0))
                .child(self.devices(theme, state.connected_devices.as_deref()));
        }
        page
    }

    fn power_row(&mut self, frame: &mut Frame, theme: &Theme, power: Option<bool>) -> Element {
        let target = f32::from(power == Some(true));
        let spring =
            self.switch.get_or_insert_with(|| Spring::new(expressive::SPATIAL_FAST, target));
        spring.set_target(target);
        let on = frame.animate(spring).clamp(0.0, 1.0);
        let touched = self.power.animate(frame);
        let fill = tile_glass(theme).lerp(theme.secondary_container.scaled(TILE_GLASS), on);
        let ink = theme.on_surface.lerp(theme.on_secondary_container, on);
        let detail = match power {
            Some(true) => "On",
            Some(false) => "Off",
            None => "No Bluetooth adapter",
        };
        let mut row = Element::row()
            .height(POWER_HEIGHT)
            .padding_xy(16.0, 0.0)
            .gap(16.0)
            .align_items(AlignItems::CENTER)
            .corner_radius(20.0)
            .background(fill.lerp(ink, state_layer(touched)))
            .child(glyph("bluetooth", ink))
            .child(
                Element::column()
                    .grow(1.0)
                    .child(label("Use Bluetooth", 18.0, 600, ink))
                    .child(label(detail, 14.0, 450, theme.on_surface_variant)),
            );
        if power.is_some() {
            row = row.target(target::BLUETOOTH_POWER).child(switch(theme, on, touched.press));
        }
        row
    }

    fn devices(&mut self, theme: &Theme, names: Option<&[String]>) -> Element {
        let names = match names {
            Some(names) if !names.is_empty() => names,
            other => {
                let message = if self.loading {
                    "Checking connections…"
                } else if other.is_none() {
                    "Device list unavailable"
                } else {
                    "No devices connected"
                };
                return Element::row()
                    .height(LIST_HEIGHT)
                    .center()
                    .corner_radius(16.0)
                    .background(tile_glass(theme))
                    .child(label(message, 14.0, 500, theme.on_surface_variant));
            }
        };
        self.scroll_y = self.scroll_y.min(max_scroll(names.len()));
        let mut rows = Element::column().width(INNER).absolute(0.0, -self.scroll_y).gap(GAP);
        for (i, name) in names.iter().enumerate() {
            let top = if i == 0 { 16.0 } else { 4.0 };
            let bottom = if i + 1 == names.len() { 16.0 } else { 4.0 };
            rows = rows.child(
                Element::row()
                    .width(INNER)
                    .height(ROW)
                    .align_items(AlignItems::CENTER)
                    .gap(16.0)
                    .padding_xy(16.0, 0.0)
                    .corner_radii(top, top, bottom, bottom)
                    .background(tile_glass(theme))
                    .child(glyph("bluetooth_connected", theme.primary))
                    .child(label(name, 16.0, 500, theme.on_surface).width(0.0).grow(1.0)),
            );
        }
        Element::new().size(INNER, LIST_HEIGHT).clip().target(target::DEVICES).child(rows)
    }
}

fn max_scroll(count: usize) -> f32 {
    (count as f32 * (ROW + GAP) - GAP - LIST_HEIGHT).max(0.0)
}

fn glyph(name: &'static str, color: Color) -> Element {
    Element::symbol(Icon::new(name).size(24.0)).text_color(color)
}

// Material SwitchTokens and AndroidX Switch.kt: the unselected handle grows when
// selected, and the pressed handle grows toward the edge of the track.
fn switch(theme: &Theme, on: f32, press: f32) -> Element {
    let rest = 16.0 + 8.0 * on;
    let size = rest + (28.0 - rest) * press.clamp(0.0, 1.0);
    let center = 16.0 + 20.0 * on;
    Element::new()
        .size(52.0, 32.0)
        .shrink(0.0)
        .corner_radius(16.0)
        .background(
            theme
                .secondary_container
                .scaled(TILE_GLASS)
                .lerp(theme.primary.scaled(ACCENT_GLASS), on),
        )
        .child(
            Element::new()
                .absolute(center - size / 2.0, (32.0 - size) / 2.0)
                .size(size, size)
                .corner_radius(size / 2.0)
                .background(theme.on_secondary_container.lerp(theme.on_primary, on).scaled(0.92)),
        )
}
