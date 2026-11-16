//! The status group: the network, the volume and the time, as a Material 3 Expressive
//! connected button group.
//!
//! Three segments sit a hair apart, rounded fully at the group's ends and only slightly
//! where they meet: the network and the volume, an outlined Material Symbol each, and
//! then the [clock](crate::clock). Beside the volume's icon is a miniature Expressive slider (an active
//! track, a handle, an inactive track with its stop) showing the level.
//!
//! A segment whose state changes speaks up, the way a pressed button in a connected group
//! does: it springs wider and turns `primary`, its inner corners round off until it's a
//! pill of its own, its icon fills in, and it says what happened (the new volume, or
//! "Ethernet", "Offline"...), while its neighbor gives up a little room. A moment later
//! it settles back. While the network is connecting its segment stays open, Material's
//! loading indicator (a shape morphing through the library as it spins) in place of the
//! icon. Offline, the segment stays in the error colors.
//!
//! Icons are Material Symbols Rounded at optical size 20 and, when light on dark,
//! grade −25, which Material recommends there. They cross-fade when they change, the new
//! one rising into place.

use std::{
    f32::consts::TAU,
    time::{Duration, Instant},
};

use shell_m3e::{
    Icon, MaterialShape, Theme, ThemeExt, ThemeMode,
    motion::{Spring, expressive, standard},
};
use shell_services::{audio::Volume, network::Network};
use shell_ui::{
    AlignItems, Color, Element, Frame, JustifyContent, PointerEvent, PointerKind, Shape,
};

/// Height of the bar's pills: Material's Small button container.
pub const HEIGHT: f32 = 40.0;
/// How long a segment that spoke up stays open.
pub const HOLD: Duration = Duration::from_millis(1800);

/// A Small button's icon.
const ICON: f32 = 20.0;
/// Space between the segments, and their corners where they meet: a connected group's.
pub const SEAM: f32 = 2.0;
pub const INNER_RADIUS: f32 = 8.0;
const OUTER_RADIUS: f32 = HEIGHT / 2.0;
/// Space between a segment's content and its end at the group's end, and at the seam.
const OUTER_PADDING: f32 = 14.0;
const INNER_PADDING: f32 = 12.0;
/// How much of its padding a segment gives up while its neighbor is open.
const SQUISH: f32 = 3.0;
/// Space between things in a segment.
const GAP: f32 = 8.0;
/// Label large, emphasized.
const LABEL_SIZE: f32 = 14.0;
const LABEL_WEIGHT: u16 = 700;
/// The icons' weight: a step up from regular, nearer the emphasized type beside them.
const WEIGHT: f32 = 500.0;
/// How far a new icon rises into place.
const RISE: f32 = 5.0;

/// What the pointer is over when it's over the volume's segment.
const VOLUME_TARGET: u32 = 20;
/// And when it's over the rest of the group.
const GROUP_TARGET: u32 = 21;
/// How much one step of the wheel over it changes the volume, and the least change worth
/// making: a touchpad turns the wheel a little at a time.
const VOLUME_STEP: f32 = 0.05;
const VOLUME_LEAST: f32 = 0.01;

/// The level slider: its length at rest and when open, and its parts.
const TRACK: f32 = 34.0;
const TRACK_OPEN: f32 = 56.0;
const TRACK_HEIGHT: f32 = 8.0;
const HANDLE: f32 = 3.0;
const HANDLE_HEIGHT: f32 = 18.0;
const HANDLE_GAP: f32 = 2.0;
/// The tracks' corners at their far ends, and where they face the handle.
const TRACK_RADIUS: f32 = TRACK_HEIGHT / 2.0;
const TRACK_INNER_RADIUS: f32 = 1.5;
/// The dot at the inactive track's end.
const STOP: f32 = 2.0;

/// The shapes Material's loading indicator morphs through, and how long each takes.
const LOADER: [MaterialShape; 7] = [
    MaterialShape::SoftBurst,
    MaterialShape::Cookie9Sided,
    MaterialShape::Pentagon,
    MaterialShape::Pill,
    MaterialShape::Sunny,
    MaterialShape::Cookie4Sided,
    MaterialShape::Oval,
];
const LOADER_STEP: f32 = 0.65;
const LOADER_SIZE: f32 = 24.0;

pub struct Status {
    network: Network,
    network_icon: Swap,
    network_pulse: Pulse,
    /// 0 → 1 as the network goes offline: its segment turns to the error colors.
    offline: Spring,
    /// 0 → 1 as it starts connecting: the loading indicator replaces the icon.
    connecting: Spring,
    /// When the loading indicator last started.
    connecting_since: Instant,

    volume: Volume,
    volume_icon: Swap,
    volume_pulse: Pulse,
    /// How far the wheel has turned the volume without its having been changed yet.
    volume_scrolled: f32,
    /// The slider's level, 0–1.
    level: Spring,
    /// 0 → 1 as the output is muted: the active track dims.
    muted: Spring,
    /// How wide the volume's number is: it grows and shrinks with its digits, rather than
    /// the segment jumping when one comes or goes.
    percent_width: Spring,
}

impl Status {
    pub fn new(network: Network, volume: Volume) -> Self {
        let flag = |on: bool| Spring::new(expressive::EFFECTS, if on { 1.0 } else { 0.0 });
        let mut status = Self {
            network,
            network_icon: Swap::new(network_icon(network)),
            network_pulse: Pulse::new(),
            offline: flag(network == Network::Disconnected),
            connecting: flag(false),
            connecting_since: Instant::now(),
            volume,
            volume_icon: Swap::new(volume_icon(volume)),
            volume_pulse: Pulse::new(),
            volume_scrolled: 0.0,
            level: Spring::new(expressive::SPATIAL, volume.level.clamp(0.0, 1.0)),
            muted: flag(volume.muted),
            percent_width: Spring::new(standard::SPATIAL, percent_width(volume))
                .with_precision(0.01),
        };
        if network == Network::Connecting {
            status.set_network(network);
        }
        status
    }

    /// Shows the new state, the segment speaking up to say so. Call [`Status::settle`]
    /// once [`HOLD`] has passed for it to close again.
    pub fn set_network(&mut self, network: Network) {
        let connecting = network == Network::Connecting;
        if connecting && self.connecting.target() == 0.0 {
            self.connecting_since = Instant::now();
        }
        // A signal's strength changing isn't news.
        let same_kind = std::mem::discriminant(&network) == std::mem::discriminant(&self.network);
        self.network = network;
        self.network_icon.set(network_icon(network));
        self.offline.set_target(if network == Network::Disconnected { 1.0 } else { 0.0 });
        self.connecting.set_target(if connecting { 1.0 } else { 0.0 });
        if connecting {
            self.network_pulse.hold();
        } else if !same_kind {
            self.network_pulse.fire();
        }
    }

    /// As [`Status::set_network`], for the volume.
    pub fn set_volume(&mut self, volume: Volume) {
        self.volume = volume;
        self.volume_icon.set(volume_icon(volume));
        self.level.set_target(volume.level.clamp(0.0, 1.0));
        self.muted.set_target(if volume.muted { 1.0 } else { 0.0 });
        self.percent_width.set_target(percent_width(volume));
        self.volume_pulse.fire();
    }

    /// Closes the segments that have been open for [`HOLD`].
    pub fn settle(&mut self) {
        let now = Instant::now();
        self.network_pulse.settle(now);
        self.volume_pulse.settle(now);
    }

    /// Takes in what the pointer did on the bar. Scrolling over the volume's segment
    /// turns it up and down: returns how much to change the volume by (of full), if at
    /// all.
    pub fn pointer_event(&mut self, event: PointerEvent) -> Option<f32> {
        if event.target != Some(VOLUME_TARGET) {
            return None;
        }
        match event.kind {
            // Up is louder.
            PointerKind::Scroll(steps) => self.volume_scrolled -= steps * VOLUME_STEP,
            PointerKind::Leave => self.volume_scrolled = 0.0,
            _ => {}
        }
        if self.volume_scrolled.abs() < VOLUME_LEAST {
            return None;
        }
        Some(std::mem::take(&mut self.volume_scrolled))
    }

    /// The group, ending in `trailing`: a segment [`HEIGHT`] tall whose leading corners
    /// are [`INNER_RADIUS`]. `led` is how much of something leads it (0–1; the tray's
    /// segments, drawn before it): that much, its first segment isn't the group's first.
    pub fn view(&mut self, frame: &mut Frame, led: f32, trailing: Element) -> Element {
        let theme = frame.theme();
        let network_open = frame.animate(&mut self.network_pulse.open);
        let volume_open = frame.animate(&mut self.volume_pulse.open);
        Element::row()
            .align_items(AlignItems::CENTER)
            .gap(SEAM)
            .target(GROUP_TARGET)
            .child(self.network_segment(frame, theme, network_open, volume_open, led))
            .child(self.volume_segment(frame, theme, volume_open, network_open))
            .child(trailing)
    }

    /// The leading segment. `open` is how far it has spoken up (it may overshoot) and
    /// `neighbor` how far the other has.
    fn network_segment(
        &mut self,
        frame: &mut Frame,
        theme: &Theme,
        open: f32,
        neighbor: f32,
        led: f32,
    ) -> Element {
        let hot = open.clamp(0.0, 1.0);
        let offline = frame.animate(&mut self.offline).clamp(0.0, 1.0);
        let connecting = frame.animate(&mut self.connecting).clamp(0.0, 1.0);
        let rest = Look::rest(theme).lerp(Look::error_rest(theme), offline);
        let active = Look::active(theme).lerp(Look::error_active(theme), offline);
        let look = rest.lerp(active, hot);

        let style = IconStyle { fill: hot, grade: grade(theme) * (1.0 - hot) };
        let mut icon = self.network_icon.view(frame, look.content.scaled(1.0 - connecting), style);
        if connecting > 0.0 {
            frame.request_frame();
            let elapsed = frame.now().saturating_duration_since(self.connecting_since);
            icon = icon.child(loader(elapsed.as_secs_f32(), look.content.scaled(connecting)));
        }

        let (text, width) = network_label(self.network);
        let inner = INNER_RADIUS + (OUTER_RADIUS - INNER_RADIUS) * hot;
        // Led, its leading end is a seam like its other.
        let leading = OUTER_RADIUS + (inner - OUTER_RADIUS) * led;
        let leading_padding = OUTER_PADDING + (seam_padding(hot, 0.0) - OUTER_PADDING) * led;
        segment(look.fill)
            .padding_sides(0.0, seam_padding(hot, neighbor), 0.0, leading_padding)
            .corner_radii(leading, inner, inner, leading)
            .child(icon)
            .child(label(text, width, open, look.content))
    }

    /// The middle segment.
    fn volume_segment(
        &mut self,
        frame: &mut Frame,
        theme: &Theme,
        open: f32,
        neighbor: f32,
    ) -> Element {
        let hot = open.clamp(0.0, 1.0);
        let look = Look::rest(theme).lerp(Look::active(theme), hot);
        let level = frame.animate(&mut self.level).clamp(0.0, 1.0);
        let muted = frame.animate(&mut self.muted).clamp(0.0, 1.0);

        let style = IconStyle { fill: hot, grade: grade(theme) * (1.0 - hot) };
        let icon = self.volume_icon.view(frame, look.content, style);
        // At rest the slider is the one spot of `primary` in the group; muted, it dims.
        let accent = theme.primary.lerp(look.content, hot);
        let accent = accent.lerp(look.content.scaled(0.45), muted);
        let length = TRACK + (TRACK_OPEN - TRACK) * open.max(0.0);

        let percent = percent(self.volume).to_string();
        let width = frame.animate(&mut self.percent_width);
        let inner = INNER_RADIUS + (OUTER_RADIUS - INNER_RADIUS) * hot;
        segment(look.fill)
            .padding_sides(0.0, seam_padding(hot, 0.0), 0.0, seam_padding(hot, neighbor))
            .corner_radius(inner)
            .target(VOLUME_TARGET)
            .child(icon)
            .child(slider(length, level, accent, look.content.scaled(0.24)))
            .child(label(&percent, width, open, look.content))
    }
}

/// Whether `event` is a click on the group: what opens the quick settings.
pub fn clicked(event: &PointerEvent) -> bool {
    event.kind == PointerKind::Click && matches!(event.target, Some(GROUP_TARGET | VOLUME_TARGET))
}

/// A segment's fill and the color of what's on it.
#[derive(Clone, Copy)]
struct Look {
    fill: Color,
    content: Color,
}

impl Look {
    /// Quiet: the pills' own fill.
    fn rest(theme: &Theme) -> Self {
        Self { fill: pill_fill(theme), content: theme.on_secondary_container }
    }

    /// Speaking up.
    fn active(theme: &Theme) -> Self {
        Self { fill: theme.primary, content: theme.on_primary }
    }

    fn error_rest(theme: &Theme) -> Self {
        Self { fill: theme.error_container, content: theme.on_error_container }
    }

    fn error_active(theme: &Theme) -> Self {
        Self { fill: theme.error, content: theme.on_error }
    }

    fn lerp(self, other: Self, t: f32) -> Self {
        Self { fill: self.fill.lerp(other.fill, t), content: self.content.lerp(other.content, t) }
    }
}

/// A color at a fraction of its opacity.
trait Scaled {
    fn scaled(self, opacity: f32) -> Self;
}

impl Scaled for Color {
    fn scaled(self, opacity: f32) -> Self {
        self.with_alpha(self.a * opacity)
    }
}

/// The bar's pills' fill: `secondary_container`, the theme's toned-down container color,
/// so they take on its hue instead of a neutral grey; translucent, so the blur behind the
/// bar shows through them as it does around them. Layers within a pill (such as the
/// workspace groups) add another veil of it.
pub fn pill_fill(theme: &Theme) -> Color {
    theme.secondary_container.with_alpha(PILL_ALPHA)
}

/// How opaque [`pill_fill`] is.
const PILL_ALPHA: f32 = 0.6;

/// Material's grade for light icons on dark backgrounds, in dark themes.
fn grade(theme: &Theme) -> f32 {
    if theme.mode == ThemeMode::Dark { -25.0 } else { 0.0 }
}

/// A segment's padding at the seam: more as it speaks up (`hot`) and its end there rounds
/// off, less while its `neighbor` is open.
fn seam_padding(hot: f32, neighbor: f32) -> f32 {
    INNER_PADDING + (OUTER_PADDING - INNER_PADDING) * hot - SQUISH * neighbor
}

/// A segment of the group, its content in a row.
fn segment(fill: Color) -> Element {
    Element::row().align_items(AlignItems::CENTER).height(HEIGHT).shrink(0.0).background(fill)
}

/// What a segment says while it's `open`: `text`, set against the end of a space `width`
/// wide, after a gap. It takes up room in step with `open` and fades in over the last of
/// it, once there's room.
fn label(text: &str, width: f32, open: f32, color: Color) -> Element {
    let room = Element::new().size(((GAP + width) * open).max(0.0), HEIGHT).shrink(0.0);
    let opacity = ((open - 0.6) / 0.4).clamp(0.0, 1.0);
    if opacity <= 0.0 {
        return room;
    }
    room.child(
        Element::row()
            .absolute(GAP, 0.0)
            .size(width, HEIGHT)
            .justify_content(JustifyContent::END)
            .child(
                Element::text(text)
                    .wrap(false)
                    .height(HEIGHT)
                    .shrink(0.0)
                    .font_size(LABEL_SIZE)
                    .font_weight(LABEL_WEIGHT)
                    .text_color(color.scaled(opacity)),
            ),
    )
}

/// A miniature Material 3 Expressive slider, `length` long after a gap, showing `level`
/// (0–1): an active track in `accent`, a handle, and the rest of the track in `inactive`
/// with a stop at its end.
fn slider(length: f32, level: f32, accent: Color, inactive: Color) -> Element {
    let top = (HEIGHT - TRACK_HEIGHT) / 2.0;
    let travel = length - HANDLE - 2.0 * HANDLE_GAP;
    let active = (travel * level).round();
    let handle = active + HANDLE_GAP;
    let rest = handle + HANDLE + HANDLE_GAP;
    let part = |x: f32, width: f32| {
        Element::new().absolute(GAP + x, top).size(width.max(0.0), TRACK_HEIGHT)
    };
    let mut slider = Element::new()
        .size(GAP + length, HEIGHT)
        .shrink(0.0)
        .child(part(0.0, active).background(accent).corner_radii(
            TRACK_RADIUS,
            TRACK_INNER_RADIUS,
            TRACK_INNER_RADIUS,
            TRACK_RADIUS,
        ))
        .child(
            Element::new()
                .absolute(GAP + handle, (HEIGHT - HANDLE_HEIGHT) / 2.0)
                .size(HANDLE, HANDLE_HEIGHT)
                .background(accent)
                .corner_radius(HANDLE / 2.0),
        )
        .child(part(rest, length - rest).background(inactive).corner_radii(
            TRACK_INNER_RADIUS,
            TRACK_RADIUS,
            TRACK_RADIUS,
            TRACK_INNER_RADIUS,
        ));
    // The stop, fading out as the handle closes in on it rather than vanishing.
    let clear = ((length - rest - TRACK_HEIGHT / 2.0) / TRACK_HEIGHT).clamp(0.0, 1.0);
    if clear > 0.0 {
        let inset = (TRACK_HEIGHT - STOP) / 2.0;
        slider = slider.child(
            Element::new()
                .absolute(GAP + length - inset - STOP, top + inset)
                .size(STOP, STOP)
                .background(accent.scaled(clear))
                .corner_radius(STOP / 2.0),
        );
    }
    slider
}

/// Material's loading indicator, `elapsed` seconds in, over an icon's slot: a shape that
/// morphs into the next of [`LOADER`] every [`LOADER_STEP`], turning as it goes.
fn loader(elapsed: f32, color: Color) -> Element {
    let steps = elapsed / LOADER_STEP;
    let step = steps.floor() as usize;
    let progress = back_out(steps.fract());
    let from = LOADER[step % LOADER.len()];
    let to = LOADER[(step + 1) % LOADER.len()];
    // A quarter turn with each morph, and a slow steady spin under it.
    let rotation = (step as f32 + progress) * TAU / 4.0 + elapsed * TAU / 8.0;
    let inset = (ICON - LOADER_SIZE) / 2.0;
    Element::new()
        .absolute(inset, inset)
        .size(LOADER_SIZE, LOADER_SIZE)
        .background(color)
        .shape(Shape::morph(from, to, progress).rotated(rotation))
}

/// Eases 0 → 1 quickly at first, overshooting a little before settling, like a spring.
fn back_out(t: f32) -> f32 {
    const OVERSHOOT: f32 = 1.70158;
    let t = t - 1.0;
    1.0 + (OVERSHOOT + 1.0) * t * t * t + OVERSHOOT * t * t
}

/// How open a segment is, and when it should close.
struct Pulse {
    /// 0 → 1 as the segment speaks up.
    open: Spring,
    /// When to close again, if it's open for a while only.
    until: Option<Instant>,
}

impl Pulse {
    fn new() -> Self {
        Self { open: Spring::new(expressive::SPATIAL, 0.0), until: None }
    }

    /// Opens for [`HOLD`].
    fn fire(&mut self) {
        self.open.set_target(1.0);
        self.until = Some(Instant::now() + HOLD);
    }

    /// Opens until fired.
    fn hold(&mut self) {
        self.open.set_target(1.0);
        self.until = None;
    }

    fn settle(&mut self, now: Instant) {
        if self.until.is_some_and(|until| now >= until) {
            self.open.set_target(0.0);
            self.until = None;
        }
    }
}

/// How an icon is drawn: how filled, and at what grade.
#[derive(Clone, Copy)]
struct IconStyle {
    fill: f32,
    grade: f32,
}

/// An icon that cross-fades into the next when it changes, the new one rising into
/// place.
struct Swap {
    current: &'static str,
    previous: Option<&'static str>,
    /// 0 → 1 as `previous` gives way to `current`.
    change: Spring,
}

impl Swap {
    fn new(name: &'static str) -> Self {
        Self { current: name, previous: None, change: Spring::new(expressive::SPATIAL, 1.0) }
    }

    fn set(&mut self, name: &'static str) {
        if name == self.current {
            return;
        }
        self.previous = Some(self.current);
        self.current = name;
        self.change.snap(0.0);
        self.change.set_target(1.0);
    }

    /// The icon in an [`ICON`] square.
    fn view(&mut self, frame: &mut Frame, color: Color, style: IconStyle) -> Element {
        let change = frame.animate(&mut self.change);
        if !self.change.is_moving() {
            self.previous = None;
        }
        let shown = change.clamp(0.0, 1.0);
        let glyph = |name: &'static str, opacity: f32, shift: f32| {
            let icon =
                Icon::new(name).size(ICON).weight(WEIGHT).fill(style.fill).grade(style.grade);
            Element::symbol(icon)
                .text_color(color.scaled(opacity))
                .absolute(0.0, 0.0)
                .offset(0.0, shift)
        };
        let mut slot = Element::new().size(ICON, ICON).shrink(0.0);
        if let Some(previous) = self.previous {
            slot = slot.child(glyph(previous, 1.0 - shown, -RISE * change));
        }
        slot.child(glyph(self.current, shown, RISE * (1.0 - change)))
    }
}

fn network_icon(network: Network) -> &'static str {
    match network {
        Network::Disconnected => "public_off",
        // Hidden under the loading indicator but for a moment.
        Network::Connecting => "wifi_find",
        Network::Wired => "lan",
        Network::Wifi { signal: 67.. } => "wifi",
        Network::Wifi { signal: 34.. } => "wifi_2_bar",
        Network::Wifi { .. } => "wifi_1_bar",
    }
}

/// What the network segment says when it speaks up, and how wide that is set in the
/// label's type.
fn network_label(network: Network) -> (&'static str, f32) {
    match network {
        Network::Disconnected => ("Offline", 46.0),
        Network::Connecting => ("Connecting", 80.0),
        Network::Wired => ("Ethernet", 59.0),
        Network::Wifi { .. } => ("Wi-Fi", 34.0),
    }
}

fn volume_icon(volume: Volume) -> &'static str {
    match volume {
        Volume { muted: true, .. } => "volume_off",
        Volume { level, .. } if level <= 0.0 => "volume_mute",
        Volume { level, .. } if level < 0.5 => "volume_down",
        _ => "volume_up",
    }
}

/// The volume as a whole percentage.
fn percent(volume: Volume) -> u32 {
    (volume.level * 100.0).round() as u32
}

/// How wide [`percent`] is set in the label's type: its digits are near enough alike.
fn percent_width(volume: Volume) -> f32 {
    const DIGIT: f32 = 8.7;
    percent(volume).to_string().len() as f32 * DIGIT
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_in_the_font() {
        let networks = [
            Network::Disconnected,
            Network::Connecting,
            Network::Wired,
            Network::Wifi { signal: 90 },
            Network::Wifi { signal: 50 },
            Network::Wifi { signal: 10 },
        ];
        let volumes = [(0.5, true), (0.0, false), (0.3, false), (0.8, false)]
            .map(|(level, muted)| Volume { level, muted });
        let names = networks.map(network_icon).into_iter().chain(volumes.map(volume_icon));
        for name in names {
            assert!(shell_m3e::icons::font().contains(name), "{name}");
        }
    }

    #[test]
    fn a_segment_opens_when_its_state_changes_and_closes_after_the_hold() {
        let mut status = Status::new(Network::Wired, Volume { level: 0.5, muted: false });
        status.set_volume(Volume { level: 0.6, muted: false });
        assert_eq!(status.volume_pulse.open.target(), 1.0);
        assert_eq!(status.network_pulse.open.target(), 0.0);
        status.volume_pulse.settle(Instant::now());
        assert_eq!(status.volume_pulse.open.target(), 1.0, "closed early");
        status.volume_pulse.settle(Instant::now() + HOLD);
        assert_eq!(status.volume_pulse.open.target(), 0.0);
    }

    #[test]
    fn the_network_segment_stays_open_while_connecting_and_ignores_signal_changes() {
        let mut status = Status::new(Network::Wired, Volume::default());
        status.set_network(Network::Connecting);
        status.network_pulse.settle(Instant::now() + HOLD);
        assert_eq!(status.network_pulse.open.target(), 1.0);
        status.set_network(Network::Wifi { signal: 80 });
        status.network_pulse.settle(Instant::now() + HOLD);
        assert_eq!(status.network_pulse.open.target(), 0.0);
        status.set_network(Network::Wifi { signal: 40 });
        assert_eq!(status.network_pulse.open.target(), 0.0);
    }

    #[test]
    fn easing_starts_at_rest_overshoots_and_lands() {
        assert!(back_out(0.0).abs() < 1e-5);
        assert!((back_out(1.0) - 1.0).abs() < 1e-5);
        assert!(back_out(0.7) > 1.0);
    }
}
