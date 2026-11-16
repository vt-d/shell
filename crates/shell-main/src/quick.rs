//! Quick settings: a panel that drops from the status group when it's clicked, after
//! Android's (as redesigned for Material 3 Expressive, and kept since).
//!
//! At the top is a slider, as tall as a tile is not: a thick track with its icon inside,
//! split by a handle. Android's is the brightness; here it is the volume. Under it are
//! the tiles, in four columns: small ones an icon alone, wide ones an icon with what it
//! switches and how that stands. A tile that's off is a quiet pill; switched on it turns
//! `primary` and squares its corners off, springing as it goes, and squares off further
//! while pressed.
//!
//! The panel grows out of the group, and is as see-through as the bar. Like the media
//! card it is two windows: itself, and a [scrim](QuickSettings::scrim) under it that
//! takes the press that closes it.

use std::path::PathBuf;

use shell_m3e::{
    Icon, Theme, ThemeExt,
    components::{Slider, Touch, Touched},
    motion::{Spring, expressive},
};
use shell_services::{
    audio::Volume,
    clock::Time,
    quick::{State, Switch},
};
use shell_ui::{
    AlignItems, Anchor, Color, Element, Extent, Frame, Image, ImageFile, JustifyContent, Layer,
    Margin, PointerEvent, PointerKind, Window, WindowId,
};

use crate::status::HEIGHT;

/// The panel.
const WIDTH: f32 = 420.0;
/// How wide it is as it starts to open: about the group's width.
const SEED: f32 = 240.0;
const RADIUS: f32 = 32.0;
const PADDING: f32 = 16.0;
const TOP_PADDING: f32 = 20.0;
/// Space between the slider and the tiles.
const SECTION_GAP: f32 = 16.0;
/// The width of what's on the panel.
const INNER: f32 = WIDTH - 2.0 * PADDING;
/// Space between the bar and the panel.
const MARGIN: f32 = 8.0;
/// How far the panel's right edge is from the screen's: the group's is as far.
const RIGHT: f32 = 64.0;
/// Room in the panel's window to either side of it, to overshoot into as it opens.
const SLACK: f32 = 24.0;

/// The header: who's logged in, and the session's buttons; and the space under it.
const HEADER: f32 = 48.0;
const HEADER_GAP: f32 = 20.0;
const AVATAR: f32 = 48.0;
const SESSION_BUTTON: f32 = 40.0;

/// The slider: its track, the handle that splits it, and the space either side of that.
const SLIDER_HEIGHT: f32 = 52.0;
const TRACK: f32 = 40.0;
const TRACK_RADIUS: f32 = 12.0;
const TRACK_INNER_RADIUS: f32 = 4.0;
const HANDLE: f32 = 4.0;
const HANDLE_GAP: f32 = 6.0;
const SLIDER_ICON: f32 = 20.0;
/// Space between the track's far end and the icon in it.
const SLIDER_ICON_INSET: f32 = 12.0;
/// How much one step of the wheel over the slider changes the volume.
const VOLUME_STEP: f32 = 0.05;

/// The tiles: four columns of small ones, an icon each; a wide one takes two columns
/// and has room to say what it is.
const TILE_GAP: f32 = 8.0;
const TILE_HEIGHT: f32 = 72.0;
const TILE_SMALL: f32 = (INNER - 3.0 * TILE_GAP) / 4.0;
const TILE_WIDE: f32 = 2.0 * TILE_SMALL + TILE_GAP;
/// A tile's corners when it's on, and while it's pressed. Off, it's a pill.
const TILE_ON_RADIUS: f32 = 20.0;
const TILE_PRESSED_RADIUS: f32 = 14.0;
const TILE_ICON: f32 = 24.0;
/// Where a wide tile's icon sits: a square this far in from the tile's end and its top.
/// A tile with a toggle of its own has it there, round when off and squared when on.
const SLOT: f32 = 56.0;
const SLOT_INSET: f32 = (TILE_HEIGHT - SLOT) / 2.0;
const SLOT_ON_RADIUS: f32 = 14.0;
/// Space between the slot and the tile's words.
const SLOT_GAP: f32 = 6.0;
/// How strongly a tile for something that isn't there shows.
const DISABLED: f32 = 0.38;

/// How opaque the panel is: glass over the compositor's blur, as Android's shade is over
/// the wallpaper. Tiles and tracks that are off are darker glass on it.
const GLASS: f32 = 0.7;
const TILE_GLASS: f32 = 0.72;
/// How far the glass takes on the theme's hue.
const TINT: f32 = 0.35;

const HEIGHT_FULL: f32 = TOP_PADDING
    + HEADER
    + HEADER_GAP
    + SLIDER_HEIGHT
    + SECTION_GAP
    + 2.0 * TILE_HEIGHT
    + TILE_GAP
    + PADDING;
const SURFACE_WIDTH: u32 = (WIDTH + 2.0 * SLACK) as u32;
const SURFACE_HEIGHT: u32 = (MARGIN + HEIGHT_FULL * 1.12) as u32;

/// What the pointer is over.
mod target {
    pub const PANEL: u32 = 40;
    pub const SLIDER: u32 = 41;
    pub const RESTART: u32 = 42;
    pub const POWER_OFF: u32 = 43;
    /// The first tile; the rest follow.
    pub const TILE: u32 = 50;
}

/// The tiles, in order.
const TILES: usize = 7;
const WIFI: usize = 0;
const BLUETOOTH: usize = 1;
const DND: usize = 2;
const AIRPLANE: usize = 3;
const MUTE: usize = 4;
const MIC: usize = 5;
const SUSPEND: usize = 6;

/// The header's buttons, in order.
const RESTART: usize = 0;
const POWER_OFF: usize = 1;

/// Something for the shell to do in answer to the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Open a window showing [`QuickSettings::scrim`] and tell
    /// [`QuickSettings::set_scrim_window`].
    OpenScrim,
    /// Open a window showing [`QuickSettings::panel`] and tell
    /// [`QuickSettings::set_panel_window`].
    OpenPanel,
    /// Find the state out and tell [`QuickSettings::set_state`].
    Query,
    /// Do this, and then find the state out again.
    Switch(Switch),
    /// Set the volume to this (0–1).
    SetVolume(f32),
}

pub struct QuickSettings {
    state: State,
    volume: Volume,
    time: Time,
    /// Who's logged in, and their picture if they have one.
    user: String,
    picture: Option<PathBuf>,
    /// How long the machine has been up, as of when the panel opened.
    uptime: Option<String>,
    session: [SessionButton; 2],
    /// The header button a click has armed, if any: another click on it does it.
    armed: Option<usize>,
    is_open: bool,
    /// 0 → 1 as the panel opens.
    open: Spring,
    panel_window: Option<WindowId>,
    scrim_window: Option<WindowId>,
    slider: Touch,
    /// The volume as last set here, shown until the system says what it is.
    volume_set: Option<f32>,
    /// The slider's level, 0–1.
    level: Spring,
    tiles: [Tile; TILES],
}

/// One of the header's buttons, animated.
struct SessionButton {
    touch: Touch,
    /// 0 → 1 as a click arms it.
    armed: Spring,
}

/// A tile's state, animated.
struct Tile {
    touch: Touch,
    /// 0 → 1 as it's switched on.
    on: Spring,
}

/// What a tile shows and does.
struct Face {
    icon: &'static str,
    title: &'static str,
    /// How what it switches stands, on a wide tile.
    detail: Option<String>,
    /// Whether it's on; `None` if what it switches isn't there.
    on: Option<bool>,
    wide: bool,
    /// Whether it has a toggle of its own, as Android's tiles that also open something
    /// do: the toggle shows how it stands, and the tile stays quiet.
    toggle: bool,
}

impl QuickSettings {
    pub fn new(volume: Volume, time: Time) -> Self {
        Self {
            state: State::default(),
            volume,
            time,
            user: user_name(),
            picture: picture(),
            uptime: uptime(),
            session: std::array::from_fn(|_| SessionButton {
                touch: Touch::new(),
                armed: Spring::new(expressive::SPATIAL_FAST, 0.0),
            }),
            armed: None,
            is_open: false,
            open: Spring::new(expressive::SPATIAL, 0.0),
            panel_window: None,
            scrim_window: None,
            slider: Touch::new(),
            volume_set: None,
            level: Spring::new(expressive::SPATIAL_FAST, volume.level.clamp(0.0, 1.0)),
            tiles: std::array::from_fn(|_| Tile {
                touch: Touch::new(),
                on: Spring::new(expressive::SPATIAL_FAST, 0.0),
            }),
        }
    }

    /// Takes in the state as it is now.
    pub fn set_state(&mut self, state: State) {
        self.state = state;
    }

    /// Takes in the time as it is now, for the date.
    pub fn set_time(&mut self, time: Time) {
        self.time = time;
    }

    /// Takes in the volume as it is now.
    pub fn set_volume(&mut self, volume: Volume) {
        self.volume = volume;
        // The system has caught up with what was set here.
        if !self.slider.held() {
            self.volume_set = None;
        }
    }

    pub fn set_panel_window(&mut self, window: WindowId) {
        self.panel_window = Some(window);
    }

    pub fn set_scrim_window(&mut self, window: WindowId) {
        self.scrim_window = Some(window);
    }

    /// The windows open for the panel, to refresh along with the bar.
    pub fn windows(&self) -> impl Iterator<Item = WindowId> + use<> {
        self.panel_window.into_iter().chain(self.scrim_window)
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Opens the panel if it's closed and closes it if it's open.
    pub fn toggle(&mut self) -> Vec<Action> {
        if self.is_open {
            self.close();
            return Vec::new();
        }
        self.is_open = true;
        self.open.set_target(1.0);
        self.uptime = uptime();
        // A panel still closing is opened again where it is.
        let mut actions = vec![Action::Query];
        if self.scrim_window.is_none() {
            actions.push(Action::OpenScrim);
        }
        if self.panel_window.is_none() {
            actions.push(Action::OpenPanel);
        }
        actions
    }

    pub fn close(&mut self) {
        self.armed = None;
        self.is_open = false;
        self.open.set_target(0.0);
    }

    /// A window over all of the screen under the bar, with nothing in it: a press on it
    /// is a press off the panel, which closes the panel.
    pub fn scrim(&mut self, frame: &mut Frame) -> Window {
        if !self.is_open {
            frame.close();
            self.scrim_window = None;
        }
        Window::layer_shell("quick-settings-scrim").width(Extent::Fill).height(Extent::Fill)
    }

    /// The panel's window: under the status group, and no bigger than the panel gets.
    pub fn panel(&mut self, frame: &mut Frame) -> Window {
        let open = frame.animate(&mut self.open);
        if !self.is_open && !self.open.is_moving() {
            frame.close();
            self.panel_window = None;
        }
        let root = Element::column()
            .align_items(AlignItems::END)
            .padding_sides(MARGIN, SLACK, 0.0, SLACK)
            .child(self.sheet(frame, open));
        // Above the scrim, whichever of the two the compositor happens to map first.
        Window::layer_shell("quick-settings")
            .layer(Layer::Overlay)
            .anchor(Anchor::TOP | Anchor::RIGHT)
            .margin(Margin::new(0, (RIGHT - SLACK) as i32, 0, 0))
            .width(Extent::Fixed(SURFACE_WIDTH))
            .height(Extent::Fixed(SURFACE_HEIGHT))
            .root(root)
    }

    /// The panel itself, `open` (0–1, overshooting) of the way out: it grows down and to
    /// the left from under the group.
    fn sheet(&mut self, frame: &mut Frame, open: f32) -> Element {
        let theme = frame.theme();
        let width = SEED + (WIDTH - SEED) * open;
        let height = (HEIGHT_FULL * open).max(0.0);
        let shown = (open * 4.0).clamp(0.0, 1.0);
        let radius = HEIGHT / 2.0 + (RADIUS - HEIGHT / 2.0) * open.clamp(0.0, 1.0);

        let faces = self.faces();
        let mut rows = Element::column().gap(TILE_GAP);
        for row in [&[WIFI, BLUETOOTH, DND][..], &[AIRPLANE, MUTE, MIC, SUSPEND]] {
            let mut tiles = Element::row().height(TILE_HEIGHT).gap(TILE_GAP);
            for &index in row {
                tiles = tiles.child(self.tile(frame, theme, index, &faces[index]));
            }
            rows = rows.child(tiles);
        }
        let content = Element::column()
            .absolute(width - WIDTH, 0.0)
            .width(WIDTH)
            .padding_sides(TOP_PADDING, PADDING, PADDING, PADDING)
            .child(self.header(frame, theme))
            .child(Element::new().height(HEADER_GAP))
            .child(self.volume_slider(frame, theme))
            .child(Element::new().height(SECTION_GAP))
            .child(rows);
        Element::new()
            .size(width, height)
            .shrink(0.0)
            // Glass: what's behind shows through the compositor's blur (a layer rule for
            // the `quick-settings` namespace).
            .background(
                theme.surface_container.lerp(theme.primary_container, TINT).scaled(shown * GLASS),
            )
            .corner_radius(radius)
            .clip()
            .target(target::PANEL)
            .child(content)
    }

    /// Who's logged in: their picture, their name, and under it the date and how long the
    /// machine has been up; across from them, restarting and powering off.
    fn header(&mut self, frame: &mut Frame, theme: &Theme) -> Element {
        let picture = match &self.picture {
            Some(path) => Element::new()
                .size(AVATAR, AVATAR)
                .shrink(0.0)
                .corner_radius(AVATAR / 2.0)
                .clip()
                .child(Element::image(Image::new(ImageFile::new(path), AVATAR).cover())),
            // No picture: their initial.
            None => Element::row()
                .center()
                .size(AVATAR, AVATAR)
                .shrink(0.0)
                .background(theme.primary)
                .corner_radius(AVATAR / 2.0)
                .child(label(&initial(&self.user), 22.0, 700, theme.on_primary)),
        };
        let detail = match &self.uptime {
            Some(uptime) => format!("{}  ·  up {uptime}", date(self.time)),
            None => date(self.time),
        };
        let words = Element::column()
            .grow(1.0)
            .width(0.0)
            .justify_content(JustifyContent::CENTER)
            .gap(1.0)
            .child(label(&self.user, 18.0, 700, theme.on_surface))
            .child(label(&detail, 13.0, 500, theme.on_surface_variant));
        Element::row()
            .align_items(AlignItems::CENTER)
            .height(HEADER)
            .gap(12.0)
            .child(picture)
            .child(words)
            .child(
                Element::row()
                    .align_items(AlignItems::CENTER)
                    .gap(8.0)
                    .child(self.session_button(frame, theme, RESTART))
                    .child(self.session_button(frame, theme, POWER_OFF)),
            )
    }

    /// One of the header's buttons: round and quiet, until a click arms it. Armed, it
    /// turns to the error colors and squares off, and a second click does it.
    fn session_button(&mut self, frame: &mut Frame, theme: &Theme, which: usize) -> Element {
        let button = &mut self.session[which];
        button.armed.set_target(if self.armed == Some(which) { 1.0 } else { 0.0 });
        let armed = frame.animate(&mut button.armed);
        let touched = button.touch.animate(frame);
        let amount = armed.clamp(0.0, 1.0);
        let content = theme.on_surface.lerp(theme.on_error, amount);
        let rest = SESSION_BUTTON / 2.0 + (12.0 - SESSION_BUTTON / 2.0) * armed;
        let radius = rest + (10.0 - rest) * touched.press.clamp(0.0, 1.0);
        let icon = [("restart_alt", target::RESTART), ("power_settings_new", target::POWER_OFF)];
        let (icon, id) = icon[which];
        Element::row()
            .center()
            .size(SESSION_BUTTON, SESSION_BUTTON)
            .shrink(0.0)
            .background(tile_glass(theme).lerp(theme.error, amount))
            .corner_radius(radius.clamp(0.0, SESSION_BUTTON / 2.0))
            .clip()
            .target(id)
            .child(
                Element::new()
                    .absolute(0.0, 0.0)
                    .size(SESSION_BUTTON, SESSION_BUTTON)
                    .background(content.scaled(state_layer(touched))),
            )
            .child(Element::symbol(Icon::new(icon).size(20.0).weight(500.0)).text_color(content))
    }

    /// What each tile shows, as things stand.
    fn faces(&self) -> [Face; TILES] {
        let state = &self.state;
        let stands = |on: Option<bool>| on.map(|on| if on { "On" } else { "Off" }.to_owned());
        let wifi = match (state.wifi, &state.ssid) {
            (Some(true), Some(ssid)) => Some(ssid.clone()),
            (on, _) => stands(on),
        };
        let muted = self.volume.muted;
        let mic_muted = state.mic_muted;
        [
            Face {
                icon: if state.wifi == Some(false) { "wifi_off" } else { "wifi" },
                title: "Wi-Fi",
                detail: wifi.or_else(|| Some("None".to_owned())),
                on: state.wifi,
                wide: false,
                toggle: false,
            },
            Face {
                icon: if state.bluetooth == Some(true) {
                    "bluetooth"
                } else {
                    "bluetooth_disabled"
                },
                title: "Bluetooth",
                detail: None,
                on: state.bluetooth,
                wide: true,
                toggle: true,
            },
            Face {
                icon: "do_not_disturb_on",
                title: "Do Not Disturb",
                detail: stands(Some(state.dnd)),
                on: Some(state.dnd),
                wide: false,
                toggle: false,
            },
            Face {
                icon: "airplanemode_active",
                title: "Airplane mode",
                detail: stands(Some(state.airplane())),
                on: (state.wifi.is_some() || state.bluetooth.is_some()).then(|| state.airplane()),
                wide: false,
                toggle: false,
            },
            Face {
                icon: if muted { "volume_off" } else { "volume_up" },
                title: "Mute",
                detail: None,
                on: Some(muted),
                wide: false,
                toggle: false,
            },
            Face {
                icon: if mic_muted == Some(false) { "mic" } else { "mic_off" },
                title: "Mute microphone",
                detail: None,
                on: mic_muted,
                wide: false,
                toggle: false,
            },
            Face {
                icon: "bedtime",
                title: "Suspend",
                detail: Some("Sleep now".to_owned()),
                on: Some(false),
                wide: false,
                toggle: false,
            },
        ]
    }

    /// The volume to show: as just set here, else the system's.
    fn shown_volume(&self) -> f32 {
        self.volume_set.unwrap_or(self.volume.level).clamp(0.0, 1.0)
    }

    /// The slider at the top: Android's, with the volume where the brightness would be.
    fn volume_slider(&mut self, frame: &mut Frame, theme: &Theme) -> Element {
        let touched = self.slider.animate(frame);
        // Dragged, the handle is where the pointer is; otherwise it springs to the level.
        let target = self.shown_volume();
        if self.slider.held() {
            self.level.snap(target);
        }
        self.level.set_target(target);
        let level = frame.animate(&mut self.level).clamp(0.0, 1.0);
        let muted = self.volume.muted;

        let held = touched.press.clamp(0.0, 1.0);
        let handle = HANDLE - 2.0 * held;
        let center = HANDLE / 2.0 + (INNER - HANDLE) * level;
        let active = (center - handle / 2.0 - HANDLE_GAP).max(0.0);
        let rest = (center + handle / 2.0 + HANDLE_GAP).min(INNER);
        let top = (SLIDER_HEIGHT - TRACK) / 2.0;
        let fill = if muted { theme.outline } else { theme.primary };

        let icon = match level {
            _ if muted => "volume_off",
            l if l <= 0.0 => "volume_mute",
            l if l < 0.5 => "volume_down",
            _ => "volume_up",
        };
        // The icon sits at the track's far end, as Android's does, taking the color of
        // what's on whichever part of the track is under it.
        let icon_x = INNER - SLIDER_ICON_INSET - SLIDER_ICON;
        let covered = active > icon_x + SLIDER_ICON / 2.0;
        let icon_color = if covered { theme.on_primary } else { theme.on_surface };
        Element::new()
            .size(INNER, SLIDER_HEIGHT)
            .shrink(0.0)
            .target(target::SLIDER)
            .child(
                Element::new()
                    .absolute(0.0, top)
                    .size(active, TRACK)
                    .background(fill)
                    .corner_radii(
                        TRACK_RADIUS,
                        TRACK_INNER_RADIUS,
                        TRACK_INNER_RADIUS,
                        TRACK_RADIUS,
                    ),
            )
            .child(
                Element::new()
                    .absolute(rest, top)
                    .size(INNER - rest, TRACK)
                    .background(tile_glass(theme))
                    .corner_radii(
                        TRACK_INNER_RADIUS,
                        TRACK_RADIUS,
                        TRACK_RADIUS,
                        TRACK_INNER_RADIUS,
                    ),
            )
            .child(
                Element::new()
                    .absolute(center - handle / 2.0, 0.0)
                    .size(handle, SLIDER_HEIGHT)
                    .background(fill)
                    .corner_radius(handle / 2.0),
            )
            .child(
                Element::symbol(Icon::new(icon).size(SLIDER_ICON).fill(1.0))
                    .text_color(icon_color)
                    .absolute(icon_x, (SLIDER_HEIGHT - SLIDER_ICON) / 2.0),
            )
    }

    /// Tile `index`, showing `face`.
    fn tile(&mut self, frame: &mut Frame, theme: &Theme, index: usize, face: &Face) -> Element {
        let tile = &mut self.tiles[index];
        tile.on.set_target(if face.on == Some(true) { 1.0 } else { 0.0 });
        let on = frame.animate(&mut tile.on);
        let touched =
            if face.on.is_some() { tile.touch.animate(frame) } else { Touched::default() };
        let amount = on.clamp(0.0, 1.0);
        let strength = if face.on.is_some() { 1.0 } else { DISABLED };
        let pressed = touched.press.clamp(0.0, 1.0);

        // A tile with a toggle of its own stays quiet, and its toggle says how it stands.
        let (body, body_on) = if face.toggle { (0.0, 0.0) } else { (amount, on) };
        let off = tile_glass(theme);
        let fill = off.lerp(theme.primary, body);
        let content = theme.on_surface.lerp(theme.on_primary, body).scaled(strength);
        let quiet = theme.on_surface_variant.lerp(theme.on_primary, body).scaled(strength);
        // The corners follow the spring, overshoot and all, and square off under a press.
        let rest = TILE_HEIGHT / 2.0 + (TILE_ON_RADIUS - TILE_HEIGHT / 2.0) * body_on;
        let radius = (rest + (TILE_PRESSED_RADIUS - rest) * pressed).clamp(0.0, TILE_HEIGHT / 2.0);
        let width = if face.wide { TILE_WIDE } else { TILE_SMALL };

        let mut element = Element::row()
            .align_items(AlignItems::CENTER)
            .size(width, TILE_HEIGHT)
            .shrink(0.0)
            .background(fill.scaled(strength))
            .corner_radius(radius)
            .clip()
            .child(
                Element::new()
                    .absolute(0.0, 0.0)
                    .size(width, TILE_HEIGHT)
                    .background(content.scaled(state_layer(touched))),
            );
        if face.on.is_some() {
            element = element.target(target::TILE + index as u32);
        }
        let glyph = |color: Color| {
            Element::symbol(Icon::new(face.icon).size(TILE_ICON).fill(amount)).text_color(color)
        };
        if !face.wide {
            return element.justify_content(JustifyContent::CENTER).child(glyph(content));
        }

        let mut slot = Element::row().center().size(SLOT, SLOT).shrink(0.0);
        slot = if face.toggle {
            let radius = SLOT / 2.0 + (SLOT_ON_RADIUS - SLOT / 2.0) * on;
            let radius = radius + (10.0 - radius) * pressed;
            let color = theme.on_surface.lerp(theme.on_primary, amount).scaled(strength);
            slot.background(
                theme.surface_container_highest.lerp(theme.primary, amount).scaled(strength),
            )
            .corner_radius(radius.clamp(0.0, SLOT / 2.0))
            .child(glyph(color))
        } else {
            slot.child(glyph(content))
        };
        let mut words = Element::column()
            .grow(1.0)
            .width(0.0)
            .justify_content(JustifyContent::CENTER)
            .child(label(face.title, 14.0, 600, content));
        if let Some(detail) = &face.detail {
            words = words.child(label(detail, 12.0, 500, quiet));
        }
        element.padding_sides(0.0, 16.0, 0.0, SLOT_INSET).gap(SLOT_GAP).child(slot).child(words)
    }

    /// Takes in what the pointer did on either of the panel's windows.
    pub fn event(&mut self, event: PointerEvent) -> Vec<Action> {
        let Some(id) = event.target else {
            // A press off the panel.
            if event.kind == PointerKind::Press {
                self.close();
            }
            return Vec::new();
        };
        match id {
            target::SLIDER => {
                self.slider.handle(event.kind);
                match event.kind {
                    // Dragging only moves the handle; the volume is set once, on letting go.
                    PointerKind::Press | PointerKind::Drag => {
                        self.volume_set = Some(Slider::value_at(&event));
                        Vec::new()
                    }
                    PointerKind::Release => {
                        self.volume_set.map(Action::SetVolume).into_iter().collect()
                    }
                    PointerKind::Scroll(steps) => {
                        // Up is louder.
                        let level = (self.shown_volume() - steps * VOLUME_STEP).clamp(0.0, 1.0);
                        self.volume_set = Some(level);
                        vec![Action::SetVolume(level)]
                    }
                    _ => Vec::new(),
                }
            }
            target::RESTART | target::POWER_OFF => {
                let which = if id == target::RESTART { RESTART } else { POWER_OFF };
                self.session[which].touch.handle(event.kind);
                match event.kind {
                    // The first click arms it and the second does it; moving off disarms it.
                    PointerKind::Click if self.armed == Some(which) => {
                        self.close();
                        let switch = [Switch::Restart, Switch::PowerOff][which];
                        vec![Action::Switch(switch)]
                    }
                    PointerKind::Click => {
                        self.armed = Some(which);
                        Vec::new()
                    }
                    PointerKind::Leave => {
                        self.armed.take_if(|armed| *armed == which);
                        Vec::new()
                    }
                    _ => Vec::new(),
                }
            }
            id if id >= target::TILE => {
                let index = (id - target::TILE) as usize;
                let Some(tile) = self.tiles.get_mut(index) else { return Vec::new() };
                tile.touch.handle(event.kind);
                if event.kind != PointerKind::Click {
                    return Vec::new();
                }
                self.switch(index).map(Action::Switch).into_iter().collect()
            }
            _ => Vec::new(),
        }
    }

    /// What clicking tile `index` switches, shown as done at once: the state found out
    /// afterwards says whether it was.
    fn switch(&mut self, index: usize) -> Option<Switch> {
        let state = &mut self.state;
        Some(match index {
            WIFI => {
                let on = !state.wifi?;
                state.wifi = Some(on);
                if !on {
                    state.ssid = None;
                }
                Switch::Wifi(on)
            }
            BLUETOOTH => {
                let on = !state.bluetooth?;
                state.bluetooth = Some(on);
                Switch::Bluetooth(on)
            }
            DND => {
                state.dnd = !state.dnd;
                Switch::DoNotDisturb(state.dnd)
            }
            AIRPLANE => {
                let on = !state.airplane();
                for radio in [&mut state.wifi, &mut state.bluetooth] {
                    *radio = radio.map(|_| !on);
                }
                if on {
                    state.ssid = None;
                }
                Switch::Airplane(on)
            }
            MUTE => {
                self.volume.muted = !self.volume.muted;
                Switch::Mute(self.volume.muted)
            }
            MIC => {
                let muted = !state.mic_muted?;
                state.mic_muted = Some(muted);
                Switch::MicMute(muted)
            }
            SUSPEND => {
                self.close();
                Switch::Suspend
            }
            _ => return None,
        })
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

/// Who's logged in: their full name if the system has one, else their login.
fn user_name() -> String {
    let login = std::env::var("USER").unwrap_or_default();
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let full = passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        (fields.next()? == login).then(|| fields.nth(3))?
    });
    // The name is the first of the comment field's comma-separated parts.
    match full.and_then(|full| full.split(',').next()).map(str::trim) {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => login,
    }
}

/// Their picture, where desktops keep it: `~/.face`, or with the accounts service.
fn picture() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let user = std::env::var("USER").unwrap_or_default();
    let places = [
        home.as_ref().map(|home| home.join(".face")),
        home.as_ref().map(|home| home.join(".face.icon")),
        Some(PathBuf::from("/var/lib/AccountsService/icons").join(user)),
    ];
    places.into_iter().flatten().find(|path| path.is_file())
}

/// What stands for someone without a picture: the first letter of their name.
fn initial(name: &str) -> String {
    name.chars().next().map(|first| first.to_uppercase().collect()).unwrap_or_default()
}

/// How long the machine has been up: "3h 12m", "2d 4h", "12m".
fn uptime() -> Option<String> {
    let uptime = std::fs::read_to_string("/proc/uptime").ok()?;
    let seconds = uptime.split_whitespace().next()?.parse::<f64>().ok()? as u64;
    Some(span(seconds))
}

fn span(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86400, seconds / 3600 % 24, seconds / 60 % 60);
    match (days, hours) {
        (0, 0) => format!("{minutes}m"),
        (0, _) => format!("{hours}h {minutes}m"),
        _ => format!("{days}d {hours}h"),
    }
}

/// What tiles and tracks that are off are made of: darker glass than the panel's.
fn tile_glass(theme: &Theme) -> Color {
    theme.surface_container_lowest.lerp(theme.primary_container, TINT / 2.0).scaled(TILE_GLASS)
}

/// The date as Android's shade gives it: "Wed, Nov 11".
fn date(time: Time) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] =
        ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let day = DAYS[usize::from(time.weekday) % 7];
    let month = MONTHS[usize::from(time.month.max(1) - 1) % 12];
    format!("{day}, {month} {}", time.day)
}

/// A state layer's opacity: Material's 8% under the pointer and 10% more when pressed.
fn state_layer(touched: Touched) -> f32 {
    0.08 * touched.hover.clamp(0.0, 1.0) + 0.10 * touched.press.clamp(0.0, 1.0)
}

/// A line of type that ends in an ellipsis if there isn't room for it.
fn label(text: &str, size: f32, weight: u16, color: Color) -> Element {
    Element::text(text).ellipsis(true).font_size(size).font_weight(weight).text_color(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(state: State) -> QuickSettings {
        let mut panel = QuickSettings::new(Volume { level: 0.5, muted: false }, Time::default());
        panel.set_state(state);
        panel
    }

    #[test]
    fn every_icon_is_in_the_font() {
        let states = [
            State::default(),
            State {
                wifi: Some(true),
                bluetooth: Some(true),
                mic_muted: Some(false),
                ..State::default()
            },
            State { wifi: Some(false), ..State::default() },
        ];
        let sliders = ["volume_off", "volume_mute", "volume_down", "volume_up"];
        for state in states {
            for muted in [false, true] {
                let mut panel = panel(state.clone());
                panel.volume.muted = muted;
                for name in panel.faces().iter().map(|face| face.icon).chain(sliders) {
                    assert!(shell_m3e::icons::font().contains(name), "{name}");
                }
            }
        }
    }

    #[test]
    fn a_tile_switches_what_it_shows_and_shows_it_done() {
        let mut panel = panel(State { wifi: Some(true), ..State::default() });
        assert_eq!(panel.switch(WIFI), Some(Switch::Wifi(false)));
        assert_eq!(panel.faces()[WIFI].on, Some(false));
        assert_eq!(panel.switch(DND), Some(Switch::DoNotDisturb(true)));
        assert_eq!(panel.switch(MUTE), Some(Switch::Mute(true)));
    }

    #[test]
    fn tiles_for_what_is_not_there_do_nothing() {
        let mut panel = panel(State::default());
        assert_eq!(panel.switch(BLUETOOTH), None);
        assert_eq!(panel.switch(MIC), None);
        assert_eq!(panel.faces()[AIRPLANE].on, None);
    }

    #[test]
    fn airplane_mode_turns_every_radio_off_and_back_on() {
        let state = State { wifi: Some(true), bluetooth: Some(false), ..State::default() };
        let mut panel = panel(state);
        assert_eq!(panel.switch(AIRPLANE), Some(Switch::Airplane(true)));
        assert!(panel.state.airplane());
        assert_eq!(panel.switch(AIRPLANE), Some(Switch::Airplane(false)));
        assert_eq!((panel.state.wifi, panel.state.bluetooth), (Some(true), Some(true)));
    }

    #[test]
    fn opening_asks_for_the_state_and_both_windows() {
        let mut panel = panel(State::default());
        assert_eq!(panel.toggle(), [Action::Query, Action::OpenScrim, Action::OpenPanel]);
        assert!(panel.is_open());
        assert!(panel.toggle().is_empty());
        assert!(!panel.is_open());
    }
}
