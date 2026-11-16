//! Now playing: a pill in the middle of the bar while music plays, and the card that
//! opens under it.
//!
//! The pill shows the player most worth showing (Spotify before other music players
//! before browsers, among those playing): the track's art on a `primary` shape, its
//! title and its artist. It drops in when something starts playing and, half a minute
//! after everything has stopped, leaves again. The shape behind the art is a circle
//! while paused and a scalloped cookie while playing, and clicks round one scallop
//! when the track changes, as the title and artist roll over to the new ones. Scrolling
//! over the pill turns the player's volume up and down.
//!
//! Clicking the pill opens the card: a `surface_container_high` sheet that grows out
//! from under the bar on the expressive spatial spring. On it:
//!
//! - the art (a rounded square while playing, relaxing into a circle when paused), the
//!   player, the title, the artist and the album;
//! - the progress, as Material's wavy track: the part played is a running wave that
//!   flattens when paused; drag anywhere along it to seek;
//! - previous, play/pause and next, the middle one a wide `primary` button that squares
//!   off while playing; and where the player has them, shuffle and repeat, toggle buttons
//!   that fill in and square off when on;
//! - the player's volume, as an Expressive slider;
//! - with more than one player about, a row to pick which is shown.
//!
//! Controls a player can't do are dimmed or left out. Clicking outside the card, or the
//! pill again, closes it.

use std::{
    f32::consts::TAU,
    time::{Duration, Instant},
};

use shell_m3e::{
    Icon, MaterialShape, Theme, ThemeExt,
    components::{Button, Slider, Touch, Touched, button, slider},
    motion::{Spring, expressive, standard},
};
use shell_services::{
    apps::AppIcons,
    mpris::{Command, Player, Repeat},
};
use shell_ui::{
    AlignItems, Anchor, Color, Element, Extent, Frame, Image, ImageFile, JustifyContent, Layer,
    PointerEvent, PointerKind, Shape, Wave, Window, WindowId,
};

use crate::{
    bar::OPACITY,
    status::{HEIGHT, pill_fill},
};

/// How long the pill stays once nothing is playing.
pub const LINGER: Duration = Duration::from_secs(30);

/// What pointer events are reported on.
mod target {
    pub const PILL: u32 = 1;
    pub const CARD: u32 = 10;
    pub const PREVIOUS: u32 = 11;
    pub const PLAY: u32 = 12;
    pub const NEXT: u32 = 13;
    pub const SHUFFLE: u32 = 14;
    pub const REPEAT: u32 = 15;
    pub const SEEK: u32 = 16;
    pub const VOLUME: u32 = 17;
    /// The first of the player switcher's chips; the rest follow.
    pub const PLAYER: u32 = 100;
}

/// The pill: the shape behind the art, the art on it, and the type.
const BACKING: f32 = 30.0;
const THUMBNAIL: f32 = 22.0;
const SCALLOPS: f32 = 9.0;
const PILL_LEADING: f32 = 5.0;
const PILL_TRAILING: f32 = 16.0;
const PILL_GAP: f32 = 10.0;
const TITLE_MAX: f32 = 240.0;
const ARTIST_MAX: f32 = 160.0;
const LABEL_SIZE: f32 = 14.0;
/// How far the title and artist travel as they roll over.
const TRAVEL: f32 = 14.0;
/// How far above its place the pill starts from.
const DROP: f32 = 56.0;
/// How much one notch of the wheel changes the volume.
const VOLUME_STEP: f32 = 0.05;
/// How far one step of the wheel over the track seeks, in seconds.
const SEEK_STEP: f32 = 5.0;

/// The card.
const CARD_WIDTH: f32 = 400.0;
/// How wide it is as it starts to open: about the pill's width.
const CARD_SEED: f32 = 260.0;
const CARD_RADIUS: f32 = 28.0;
const CARD_PADDING: f32 = 20.0;
const CARD_GAP: f32 = 18.0;
/// The width of what's on the card.
const INNER: f32 = CARD_WIDTH - 2.0 * CARD_PADDING;
/// Space between the bar and the card.
const CARD_MARGIN: f32 = 8.0;
/// The card's window: room for the card at its biggest, and to overshoot as it opens.
const SURFACE_WIDTH: u32 = (CARD_WIDTH + 48.0) as u32;
const SURFACE_HEIGHT: u32 = (CARD_MARGIN + CARD_MAX_HEIGHT * 1.12) as u32;
/// The player tabs: a connected button group.
const TAB_HEIGHT: f32 = 36.0;
const TAB_SEAM: f32 = 2.0;
const TAB_INNER_RADIUS: f32 = 8.0;
/// The art, and the scalloped shape that shows round it while playing.
const ART: f32 = 144.0;
const ART_BACKING: f32 = 172.0;
/// Under the art: some space, the title, and who it's by.
const ART_GAP: f32 = 20.0;
const TITLE_HEIGHT: f32 = 30.0;
const BYLINE_HEIGHT: f32 = 22.0;
/// Above the art, room for its backing clear of what's over it.
const ART_TOP: f32 = 6.0;
const HERO: f32 = ART_TOP + ART + ART_GAP + TITLE_HEIGHT + BYLINE_HEIGHT;
/// How fast that shape turns while playing, in radians a second.
const HERO_SPIN: f32 = 0.25;
const SEEK_HEIGHT: f32 = 24.0;
const TIMES_HEIGHT: f32 = 16.0;
const TRANSPORT_HEIGHT: f32 = 64.0;
/// How much wider a transport button gets while pressed, its neighbors making room.
const PRESS_WIDEN: f32 = 16.0;
const VOLUME_HEIGHT: f32 = 28.0;
/// The wavy track: its stroke, how far it swings and how long a wave is, and how many
/// waves pass a point each second.
const WAVE_STROKE: f32 = 4.0;
const WAVE_AMPLITUDE: f32 = 3.0;
const WAVE_LENGTH: f32 = 26.0;
const WAVE_SPEED: f32 = 0.8;
/// How long each frame of the card's ambient motion (the wave running, the art's backing
/// turning) stays up. Both move well under a pixel in this time, so they look no
/// different for being drawn at half a display's usual rate, and cost half as much.
const AMBIENT_FRAME: Duration = Duration::from_millis(32);
/// The seek handle, and the space between it and the track still to play.
const THUMB: f32 = 4.0;
const THUMB_HEIGHT: f32 = 20.0;
const THUMB_GAP: f32 = 6.0;
/// How strongly a control the player can't do shows.
const DISABLED: f32 = 0.38;

/// Something for the shell to do in answer to the pointer.
pub enum Action {
    /// Open a window showing [`Media::scrim`] and tell [`Media::set_scrim_window`].
    OpenScrim,
    /// Open a window showing [`Media::card`] and tell [`Media::set_card_window`].
    OpenCard,
    Send(Box<Player>, Command),
}

pub struct Media {
    players: Vec<Player>,
    /// The player shown, as it is now.
    current: Option<Player>,
    /// The last player shown, kept to draw while the pill leaves.
    display: Option<Player>,
    /// The player picked in the card, shown whether or not it's playing.
    pinned: Option<String>,
    /// Since when nothing has been playing.
    quiet_since: Option<Instant>,
    /// 0 → 1 as the pill drops in.
    presence: Spring,
    /// 0 → 1 as playback starts.
    playing: Spring,
    /// How many scallops round the art's backing has turned: one per track.
    turn: Spring,
    /// The title and artist rolling out, if the track just changed.
    previous_track: Option<(String, String)>,
    /// 0 → 1 as they give way to the new ones.
    roll: Spring,
    pill: Touch,

    card_window: Option<WindowId>,
    scrim_window: Option<WindowId>,
    card_open: bool,
    /// 0 → 1 as the card opens.
    open: Spring,
    /// The card's height, which depends on what the player can do.
    height: Spring,
    /// When the wave started running, to tell how far along it is.
    epoch: Instant,
    seek: Touch,
    /// Where the seek handle is being dragged to, as a fraction of the track.
    seeking: Option<f32>,
    /// Where it was let go, shown until the player says where it is.
    sought: Option<f32>,
    volume: Touch,
    /// The volume as last set here, shown until the player says what it is.
    volume_set: Option<f32>,
    previous: Touch,
    play: Touch,
    next: Touch,
    shuffle: Touch,
    repeat: Touch,
    /// 0 → 1 as shuffle and repeat are turned on.
    shuffling: Spring,
    repeating: Spring,
    chips: Vec<Touch>,
    /// How far the wheel has turned over the tabs since it last switched player.
    tabs_scrolled: f32,
    /// Which tab is selected, by index; fractions are on the way between two.
    tab: Spring,
    /// How far the backing of the card's art has turned, and when it last did.
    spin: f32,
    spun_at: Option<Instant>,
    icons: AppIcons,
}

impl Media {
    pub fn new(players: Vec<Player>) -> Self {
        let mut media = Self {
            players: Vec::new(),
            current: None,
            display: None,
            pinned: None,
            quiet_since: None,
            presence: Spring::new(expressive::SPATIAL, 0.0),
            playing: Spring::new(expressive::SPATIAL, 0.0),
            turn: Spring::new(expressive::SPATIAL, 0.0),
            previous_track: None,
            roll: Spring::new(expressive::SPATIAL, 1.0),
            pill: Touch::new(),
            card_window: None,
            scrim_window: None,
            card_open: false,
            open: Spring::new(expressive::SPATIAL, 0.0),
            height: Spring::new(standard::SPATIAL, 0.0).with_precision(0.01),
            epoch: Instant::now(),
            seek: Touch::new(),
            seeking: None,
            sought: None,
            volume: Touch::new(),
            volume_set: None,
            previous: Touch::new(),
            play: Touch::new(),
            next: Touch::new(),
            shuffle: Touch::new(),
            repeat: Touch::new(),
            shuffling: Spring::new(expressive::SPATIAL_FAST, 0.0),
            repeating: Spring::new(expressive::SPATIAL_FAST, 0.0),
            chips: Vec::new(),
            tabs_scrolled: 0.0,
            tab: Spring::new(expressive::SPATIAL_FAST, 0.0),
            spin: 0.0,
            spun_at: None,
            icons: AppIcons::default(),
        };
        media.set_players(players);
        media
    }

    /// Takes in the players as they are now.
    pub fn set_players(&mut self, players: Vec<Player>) {
        self.players = players;
        self.chips.resize_with(self.players.len(), Touch::new);
        // The player has caught up with what was set here.
        self.sought = None;
        if !self.volume.held() {
            self.volume_set = None;
        }
        self.choose();
    }

    /// Whether anything is playing; if not, call [`Media::expire`] once [`LINGER`] has
    /// passed.
    pub fn quiet(&self) -> bool {
        self.quiet_since.is_some()
    }

    /// Lets the pill go if nothing has played for [`LINGER`].
    pub fn expire(&mut self) {
        self.choose();
    }

    /// The window showing the card, once the shell has opened it.
    pub fn set_card_window(&mut self, window: WindowId) {
        self.card_window = Some(window);
    }

    /// The window showing the scrim, once the shell has opened it.
    pub fn set_scrim_window(&mut self, window: WindowId) {
        self.scrim_window = Some(window);
    }

    /// The windows open for the card, to refresh along with the bar.
    pub fn card_windows(&self) -> impl Iterator<Item = WindowId> + use<> {
        self.card_window.into_iter().chain(self.scrim_window)
    }

    /// Picks the player to show and starts animating toward it.
    fn choose(&mut self) {
        let now = Instant::now();
        if self.players.iter().any(Player::playing) {
            self.quiet_since = None;
        } else if self.quiet_since.is_none() {
            self.quiet_since = Some(now);
        }
        let find = |bus: &str| self.players.iter().find(|player| player.bus == bus);
        if self.pinned.as_deref().is_some_and(|bus| find(bus).is_none()) {
            self.pinned = None;
        }
        // A paused player stays while the card is open, and for a while after pausing.
        let lingering = self.card_open
            || self.quiet_since.is_none_or(|since| now.saturating_duration_since(since) < LINGER);
        let pick = self
            .pinned
            .as_deref()
            .and_then(find)
            .or_else(|| self.players.iter().find(|player| player.playing()))
            .or_else(|| self.current.as_ref().filter(|_| lingering).and_then(|p| find(&p.bus)))
            .cloned();

        if let (Some(old), Some(new)) = (&self.current, &pick)
            && !new.same_track(old)
        {
            self.previous_track = Some((old.title.clone(), old.artist.clone()));
            self.roll.snap(0.0);
            self.roll.set_target(1.0);
            self.turn.set_target(self.turn.target() + 1.0);
        }
        let on = |on: bool| if on { 1.0 } else { 0.0 };
        self.presence.set_target(on(pick.is_some()));
        if let Some(player) = &pick {
            self.playing.set_target(on(player.playing()));
            self.shuffling.set_target(on(player.shuffle == Some(true)));
            self.repeating.set_target(on(player.repeat.is_some_and(|r| r != Repeat::Off)));
            let height = card_height(player);
            // The first card opens at its height rather than growing to it.
            if self.display.is_none() {
                self.height.snap(height);
            }
            self.height.set_target(height);
            self.display = pick.clone();
        } else {
            self.close_card();
        }
        self.current = pick;
    }

    /// Whether the card is open (or opening).
    pub fn card_is_open(&self) -> bool {
        self.card_open
    }

    /// Closes the card, if it's open.
    pub fn dismiss(&mut self) {
        if self.card_open {
            self.close_card();
        }
    }

    fn close_card(&mut self) {
        self.card_open = false;
        self.open.set_target(0.0);
        if self.pinned.take().is_some() {
            self.choose();
        }
    }

    // --- The pill ---------------------------------------------------------------

    /// The pill, or nothing while there's nothing to show.
    pub fn pill(&mut self, frame: &mut Frame) -> Element {
        let theme = frame.theme();
        let presence = frame.animate(&mut self.presence);
        if self.current.is_none() && !self.presence.is_moving() {
            self.display = None;
        }
        let Some(player) = self.display.clone() else { return Element::new() };

        let touched = self.pill.animate(frame);
        let playing = frame.animate(&mut self.playing);
        let turn = frame.animate(&mut self.turn);
        if !self.turn.is_moving() && turn >= SCALLOPS {
            self.turn.snap(turn - SCALLOPS);
        }
        let roll = frame.animate(&mut self.roll);
        if !self.roll.is_moving() {
            self.previous_track = None;
        }

        // A circle the art covers while paused; scallops showing round it while playing.
        let shape = Shape::morph(MaterialShape::Circle, MaterialShape::Cookie9Sided, playing)
            .rotated(turn * TAU / SCALLOPS)
            .scaled(THUMBNAIL / BACKING + (1.0 - THUMBNAIL / BACKING) * playing);
        let inset = (BACKING - THUMBNAIL) / 2.0;
        let thumbnail = match &player.art {
            Some(path) => art(path, THUMBNAIL, THUMBNAIL / 2.0, 1.0),
            None => Element::symbol(Icon::new("music_note").size(16.0).fill(1.0))
                .size(THUMBNAIL, THUMBNAIL)
                .text_color(theme.on_primary),
        };
        let backing = Element::new()
            .size(BACKING, BACKING)
            .shrink(0.0)
            .background(theme.primary)
            .shape(shape)
            .child(thumbnail.absolute(inset, inset));

        let words = |title: &str, artist: &str, opacity: f32| {
            let mut row = Element::row().align_items(AlignItems::CENTER).height(HEIGHT).gap(8.0);
            row = row.child(
                label(title, LABEL_SIZE, 700, theme.on_secondary_container.scaled(opacity))
                    .max_width(TITLE_MAX),
            );
            if !artist.is_empty() {
                row = row.child(
                    label(artist, LABEL_SIZE, 500, theme.on_surface_variant.scaled(opacity))
                        .max_width(ARTIST_MAX),
                );
            }
            row
        };
        let mut text = Element::row().height(HEIGHT);
        let title = if player.title.is_empty() { &player.identity } else { &player.title };
        match &self.previous_track {
            Some((old_title, old_artist)) => {
                let (position, shown) = (roll.max(0.0), roll.clamp(0.0, 1.0));
                text = text
                    .child(
                        words(old_title, old_artist, 1.0 - (shown * 1.5).min(1.0))
                            .absolute(0.0, 0.0)
                            .offset(0.0, -TRAVEL * position),
                    )
                    .child(
                        words(title, &player.artist, shown).offset(0.0, TRAVEL * (1.0 - position)),
                    );
            }
            None => text = text.child(words(title, &player.artist, 1.0)),
        }

        // Pressed, it squares off a little, as Expressive buttons do.
        let radius = HEIGHT / 2.0 - 8.0 * touched.press.clamp(0.0, 1.0);
        let layer = theme.on_secondary_container.scaled(state_layer(touched));
        Element::row()
            .align_items(AlignItems::CENTER)
            .height(HEIGHT)
            .shrink(0.0)
            .padding_sides(0.0, PILL_TRAILING, 0.0, PILL_LEADING)
            .gap(PILL_GAP)
            .background(pill_fill(theme))
            .corner_radius(radius)
            .clip()
            .target(target::PILL)
            .offset(0.0, -DROP * (1.0 - presence))
            .child(Element::new().absolute(0.0, 0.0).fill().background(layer))
            .child(backing)
            .child(text)
    }

    /// Takes in what the pointer did on the bar.
    pub fn pill_event(&mut self, event: PointerEvent) -> Vec<Action> {
        if event.target != Some(target::PILL) {
            return Vec::new();
        }
        self.pill.handle(event.kind);
        match event.kind {
            PointerKind::Click if self.card_open => {
                self.close_card();
                Vec::new()
            }
            PointerKind::Click if self.current.is_some() => {
                self.card_open = true;
                self.open.set_target(1.0);
                // A card still closing is opened again where it is.
                let mut actions = Vec::new();
                if self.scrim_window.is_none() {
                    actions.push(Action::OpenScrim);
                }
                if self.card_window.is_none() {
                    actions.push(Action::OpenCard);
                }
                actions
            }
            PointerKind::Scroll(steps) => self.nudge_volume(-steps * VOLUME_STEP),
            _ => Vec::new(),
        }
    }

    /// Turns the player's volume up or down by `change`, if it has one.
    fn nudge_volume(&mut self, change: f32) -> Vec<Action> {
        let Some(player) = self.current.clone() else { return Vec::new() };
        let Some(volume) = self.shown_volume(&player) else { return Vec::new() };
        self.set_volume(player, (volume + change).clamp(0.0, 1.0))
    }

    fn set_volume(&mut self, player: Player, volume: f32) -> Vec<Action> {
        self.volume_set = Some(volume);
        vec![Action::Send(Box::new(player), Command::Volume(volume as f64))]
    }

    /// The volume to show: as just set here, else the player's. `None` for players
    /// without one of their own (and browsers, which report one but don't heed it).
    fn shown_volume(&self, player: &Player) -> Option<f32> {
        let volume = player.volume.filter(|_| has_volume(player))?;
        Some(self.volume_set.unwrap_or(volume as f32).clamp(0.0, 1.0))
    }

    // --- The card ---------------------------------------------------------------

    /// A window over all of the screen under the bar, with nothing in it: a press on it
    /// is a press off the card, which closes the card. It is a window of its own so that
    /// the card's, which is drawn again for every frame of an animation, can be small.
    pub fn scrim(&mut self, frame: &mut Frame) -> Window {
        if !self.card_open {
            frame.close();
            self.scrim_window = None;
        }
        Window::layer_shell("media-scrim").width(Extent::Fill).height(Extent::Fill)
    }

    /// The card's window: under the pill, and no bigger than the card can get.
    pub fn card(&mut self, frame: &mut Frame) -> Window {
        let open = frame.animate(&mut self.open);
        if !self.card_open && !self.open.is_moving() {
            frame.close();
            self.card_window = None;
        }
        let mut root = Element::column().align_items(AlignItems::CENTER).padding_sides(
            CARD_MARGIN,
            0.0,
            0.0,
            0.0,
        );
        if let Some(player) = self.display.clone() {
            root = root.child(self.sheet(frame, &player, open));
        }
        // Above the scrim, whichever of the two the compositor happens to map first.
        Window::layer_shell("media-card")
            .layer(Layer::Overlay)
            .anchor(Anchor::TOP)
            .width(Extent::Fixed(SURFACE_WIDTH))
            .height(Extent::Fixed(SURFACE_HEIGHT))
            .root(root)
    }

    /// The card itself, `open` (0–1, overshooting) of the way out.
    fn sheet(&mut self, frame: &mut Frame, player: &Player, open: f32) -> Element {
        let theme = frame.theme();
        let full = frame.animate(&mut self.height);
        let width = CARD_SEED + (CARD_WIDTH - CARD_SEED) * open;
        let height = (full * open).max(0.0);
        let shown = (open * 4.0).clamp(0.0, 1.0);
        let radius = HEIGHT / 2.0 + (CARD_RADIUS - HEIGHT / 2.0) * open.clamp(0.0, 1.0);

        let mut content = Element::column()
            .absolute((width - CARD_WIDTH) / 2.0, 0.0)
            .width(CARD_WIDTH)
            .padding(CARD_PADDING)
            .gap(CARD_GAP);
        content = content.child(self.tabs(frame, theme, player));
        content = content
            .child(self.hero(frame, theme, player))
            .child(self.progress(frame, theme, player))
            .child(self.transport(frame, theme, player));
        if let Some(volume) = self.shown_volume(player) {
            content = content.child(self.volume_row(frame, theme, volume));
        }
        Element::new()
            .size(width, height)
            .shrink(0.0)
            // As see-through as the bar, so that what's behind shows through the
            // compositor's blur (a layer rule for the `media-card` namespace).
            .background(theme.surface_container.scaled(shown * OPACITY))
            .corner_radius(radius)
            .clip()
            .target(target::CARD)
            .child(content)
    }

    /// A tab for each player, as a connected button group: the one shown is filled in
    /// and rounds off into a pill of its own.
    fn tabs(&mut self, frame: &mut Frame, theme: &Theme, shown: &Player) -> Element {
        let players = self.players.clone();
        let index = players.iter().position(|player| player.bus == shown.bus).unwrap_or(0);
        self.tab.set_target(index as f32);
        let tab = frame.animate(&mut self.tab);
        let last = players.len().saturating_sub(1);
        let outer = TAB_HEIGHT / 2.0;

        // One player has nothing to switch to: it gets a label of its own width, in the
        // tabs' place.
        let single = players.len() < 2;
        let mut row =
            Element::row().height(TAB_HEIGHT).gap(TAB_SEAM).justify_content(JustifyContent::CENTER);
        for (index, player) in players.iter().enumerate() {
            let Some(touch) = self.chips.get_mut(index) else { break };
            let touched = touch.animate(frame);
            // How far this is the one shown, as the selection slides between tabs.
            let selected = (1.0 - (tab - index as f32).abs()).clamp(0.0, 1.0);
            let fill = theme.surface_container_highest.lerp(theme.secondary_container, selected);
            let content = theme.on_surface_variant.lerp(theme.on_secondary_container, selected);
            let inner = TAB_INNER_RADIUS + (outer - TAB_INNER_RADIUS) * selected;
            let inner = inner - (inner - 4.0) * touched.press.clamp(0.0, 1.0);
            let left = if index == 0 { outer } else { inner };
            let right = if index == last { outer } else { inner };

            let mut chip = Element::row()
                .center()
                .height(TAB_HEIGHT)
                .padding_xy(16.0, 0.0)
                .gap(8.0)
                .background(fill)
                .corner_radii(left, right, right, left)
                .clip();
            if !single {
                chip = chip.grow(1.0).width(0.0).target(target::PLAYER + index as u32).child(
                    Element::new()
                        .absolute(0.0, 0.0)
                        .fill()
                        .background(content.scaled(state_layer(touched))),
                );
            }
            if let Some(icon) = self.icons.get(app_of(player)) {
                chip = chip.child(Element::image(Image::new(ImageFile::new(icon), 18.0)));
            }
            let weight = 500 + (200.0 * selected) as u16;
            row = row.child(chip.child(label(&display_name(player), 13.0, weight, content)));
        }
        row
    }

    /// The art on its turning, scalloped backing, and under it the title and who it's
    /// by.
    fn hero(&mut self, frame: &mut Frame, theme: &Theme, player: &Player) -> Element {
        let playing = frame.animate(&mut self.playing);
        // The backing turns only while playing, and keeps its angle when paused.
        let now = frame.now();
        let elapsed = self
            .spun_at
            .replace(now)
            .map_or(0.0, |last| now.saturating_duration_since(last).as_secs_f32().min(0.1));
        self.spin = (self.spin + elapsed * HERO_SPIN * playing.clamp(0.0, 1.0)) % TAU;

        // Paused, a circle hidden behind the art; playing, scallops showing round it.
        let shape = Shape::morph(MaterialShape::Circle, MaterialShape::Cookie12Sided, playing)
            .rotated(self.spin)
            .scaled(ART / ART_BACKING + (1.0 - ART / ART_BACKING) * playing);
        let picture = match &player.art {
            Some(path) => art(path, ART, ART / 2.0, 1.0),
            None => Element::row()
                .center()
                .size(ART, ART)
                .background(theme.primary_container)
                .corner_radius(ART / 2.0)
                .child(
                    Element::symbol(Icon::new("music_note").size(64.0).fill(1.0))
                        .text_color(theme.on_primary_container),
                ),
        };
        // The backing spills out round the art, into the space about it.
        let spill = (ART_BACKING - ART) / 2.0;
        let backing = Element::new()
            .size(ART, ART)
            .shrink(0.0)
            .child(
                Element::new()
                    .absolute(-spill, -spill)
                    .size(ART_BACKING, ART_BACKING)
                    .background(theme.primary)
                    .shape(shape),
            )
            .child(picture.absolute(0.0, 0.0));

        let title = if player.title.is_empty() { "Nothing playing" } else { &player.title };
        let mut by = player.artist.clone();
        if !player.album.is_empty() && player.album != player.title {
            if !by.is_empty() {
                by.push_str("  ·  ");
            }
            by.push_str(&player.album);
        }
        let line = |height: f32, text: Element| {
            Element::row().center().width(INNER).height(height).child(text)
        };
        Element::column()
            .align_items(AlignItems::CENTER)
            .height(HERO)
            .padding_sides(ART_TOP, 0.0, 0.0, 0.0)
            .child(backing)
            .child(Element::new().height(ART_GAP))
            .child(line(TITLE_HEIGHT, label(title, 22.0, 800, theme.on_surface)))
            .child(line(BYLINE_HEIGHT, label(&by, 14.0, 600, theme.primary)))
    }

    /// The wavy track, and under its ends the time so far and the track's length.
    fn progress(&mut self, frame: &mut Frame, theme: &Theme, player: &Player) -> Element {
        let touched = self.seek.animate(frame);
        let playing = frame.animate(&mut self.playing).clamp(0.0, 1.0);
        let now = frame.now();
        let position = player.position(now);
        let fraction = self.seeking.or(self.sought).unwrap_or_else(|| match player.length {
            Some(length) if !length.is_zero() => {
                (position.as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0)
            }
            _ => 0.0,
        });
        let position = match (self.seeking.or(self.sought), player.length) {
            (Some(fraction), Some(length)) => length.mul_f32(fraction),
            _ => position,
        };
        // The wave runs, and the time advances, only while playing and on show: slowly
        // enough to be drawn less often than the display refreshes.
        if player.playing() && self.card_open {
            frame.request_frame_after(AMBIENT_FRAME);
        }

        let width = INNER;
        let seekable = player.can_seek && player.length.is_some();
        let held = touched.press.clamp(0.0, 1.0);
        let thumb = THUMB + 2.0 * held;
        let center = THUMB / 2.0 + (width - THUMB) * fraction;
        let played = (center - thumb / 2.0 - if seekable { THUMB_GAP } else { 0.0 }).max(0.0);
        let rest = if seekable { center + thumb / 2.0 + THUMB_GAP } else { center }.min(width);
        let elapsed = now.saturating_duration_since(self.epoch).as_secs_f32();
        let wave_height = 2.0 * WAVE_AMPLITUDE + WAVE_STROKE + 2.0;

        let mut track = Element::new().size(width, SEEK_HEIGHT).shrink(0.0);
        if played > WAVE_STROKE {
            track = track.child(
                Element::new()
                    .absolute(0.0, (SEEK_HEIGHT - wave_height) / 2.0)
                    .size(played, wave_height)
                    .background(theme.primary)
                    .wave(Wave {
                        amplitude: WAVE_AMPLITUDE * playing,
                        wavelength: WAVE_LENGTH,
                        // Negative, so the wave runs toward the handle.
                        phase: -elapsed * WAVE_SPEED * TAU,
                        thickness: WAVE_STROKE,
                    }),
            );
        }
        track = track.child(
            Element::new()
                .absolute(rest, (SEEK_HEIGHT - WAVE_STROKE) / 2.0)
                .size((width - rest).max(0.0), WAVE_STROKE)
                .background(theme.surface_container_highest)
                .corner_radius(WAVE_STROKE / 2.0),
        );
        if seekable {
            let tall = THUMB_HEIGHT + 4.0 * held;
            track = track.target(target::SEEK).child(
                Element::new()
                    .absolute(center - thumb / 2.0, (SEEK_HEIGHT - tall) / 2.0)
                    .size(thumb, tall)
                    .background(theme.primary)
                    .corner_radius(thumb / 2.0),
            );
        }

        let time = |text: String| label(&text, 12.0, 600, theme.on_surface_variant);
        let length = player.length.map_or_else(|| "–:––".to_owned(), clock_time);
        Element::column().child(track).child(
            Element::row()
                .height(TIMES_HEIGHT)
                .align_items(AlignItems::END)
                .justify_content(JustifyContent::SPACE_BETWEEN)
                .child(time(clock_time(position)))
                .child(time(length)),
        )
    }

    /// Shuffle, previous, play/pause, next, repeat. The three in the middle are a button
    /// group: the one pressed widens, and the others make room.
    fn transport(&mut self, frame: &mut Frame, theme: &Theme, player: &Player) -> Element {
        let playing = frame.animate(&mut self.playing);
        let previous = self.previous.animate(frame);
        let play = self.play.animate(frame);
        let next = self.next.animate(frame);
        let widen = |touched: Touched, enabled: bool| {
            if enabled { PRESS_WIDEN * touched.press.clamp(0.0, 1.2) } else { 0.0 }
        };
        let (wider_previous, wider_play, wider_next) =
            (widen(previous, player.can_previous), widen(play, true), widen(next, player.can_next));

        let mut row = Element::row()
            .align_items(AlignItems::CENTER)
            .justify_content(JustifyContent::CENTER)
            .height(TRANSPORT_HEIGHT)
            .gap(8.0);

        // A toggle: quiet and round when off, filled and squared off when on.
        let toggle = |id: u32, icon: &'static str, on: f32, touched: Touched| {
            let amount = on.clamp(0.0, 1.0);
            let content = theme.on_surface_variant.lerp(theme.on_secondary, amount);
            let look = Button {
                width: 44.0,
                height: 44.0,
                fill: theme.surface_container_highest.lerp(theme.secondary, amount),
                content,
                squared: on,
            };
            let icon = Icon::new(icon).size(20.0).weight(500.0).fill(amount);
            button(id, look, touched, Element::symbol(icon).text_color(content))
        };
        // A tonal button, dimmed and deaf when the player can't do what it's for.
        let step = |id: u32, icon: &'static str, enabled: bool, width: f32, touched: Touched| {
            let strength = if enabled { 1.0 } else { DISABLED };
            let content = theme.on_secondary_container.scaled(strength);
            let look = Button {
                width,
                height: 56.0,
                fill: theme.secondary_container.scaled(strength),
                content,
                squared: 0.0,
            };
            let touched = if enabled { touched } else { Touched::default() };
            let icon = Icon::new(icon).size(26.0).fill(1.0);
            button(id, look, touched, Element::symbol(icon).text_color(content))
        };

        if player.shuffle.is_some() {
            let on = frame.animate(&mut self.shuffling);
            row = row.child(toggle(target::SHUFFLE, "shuffle", on, self.shuffle.animate(frame)));
        }
        let width = 60.0 + wider_previous - wider_play / 2.0;
        row = row.child(step(
            target::PREVIOUS,
            "skip_previous",
            player.can_previous,
            width,
            previous,
        ));

        // The hero: a pill while paused, squaring off as it starts to play, its icon
        // changing with it.
        let amount = playing.clamp(0.0, 1.0);
        let glyph = |name: &'static str, opacity: f32| {
            Element::symbol(Icon::new(name).size(32.0).fill(1.0))
                .text_color(theme.on_primary.scaled(opacity))
                .absolute(0.0, 0.0)
        };
        let icon = Element::new()
            .size(32.0, 32.0)
            .child(glyph("play_arrow", 1.0 - amount))
            .child(glyph("pause", amount));
        let look = Button {
            width: 108.0 + wider_play - (wider_previous + wider_next) / 2.0,
            height: TRANSPORT_HEIGHT,
            fill: theme.primary,
            content: theme.on_primary,
            squared: playing,
        };
        row = row.child(button(target::PLAY, look, play, icon));

        let width = 60.0 + wider_next - wider_play / 2.0;
        row = row.child(step(target::NEXT, "skip_next", player.can_next, width, next));
        if let Some(repeat) = player.repeat {
            let on = frame.animate(&mut self.repeating);
            let icon = if repeat == Repeat::One { "repeat_one" } else { "repeat" };
            row = row.child(toggle(target::REPEAT, icon, on, self.repeat.animate(frame)));
        }
        row
    }

    /// The player's volume: quieter than the rest, in the secondary tones.
    fn volume_row(&mut self, frame: &mut Frame, theme: &Theme, volume: f32) -> Element {
        const ICON: f32 = 20.0;
        const GAP: f32 = 12.0;
        let icon = match volume {
            v if v <= 0.0 => "volume_mute",
            v if v < 0.5 => "volume_down",
            _ => "volume_up",
        };
        let look = Slider {
            width: INNER - ICON - GAP,
            height: VOLUME_HEIGHT,
            track: 8.0,
            active: theme.secondary,
            inactive: theme.surface_container_highest,
        };
        Element::row()
            .align_items(AlignItems::CENTER)
            .height(VOLUME_HEIGHT)
            .gap(GAP)
            .child(
                Element::symbol(Icon::new(icon).size(ICON).fill(1.0))
                    .text_color(theme.on_surface_variant),
            )
            .child(slider(target::VOLUME, look, volume, self.volume.animate(frame)))
    }

    /// Takes in what the pointer did on the card's window.
    pub fn card_event(&mut self, event: PointerEvent) -> Vec<Action> {
        let Some(id) = event.target else {
            // A press off the card.
            if event.kind == PointerKind::Press {
                self.close_card();
            }
            return Vec::new();
        };
        let Some(player) = self.current.clone() else { return Vec::new() };
        let clicked = event.kind == PointerKind::Click;
        let send = |command: Command| vec![Action::Send(Box::new(player.clone()), command)];
        match id {
            target::PREVIOUS => {
                self.previous.handle(event.kind);
                if clicked && player.can_previous { send(Command::Previous) } else { Vec::new() }
            }
            target::PLAY => {
                self.play.handle(event.kind);
                if clicked { send(Command::PlayPause) } else { Vec::new() }
            }
            target::NEXT => {
                self.next.handle(event.kind);
                if clicked && player.can_next { send(Command::Next) } else { Vec::new() }
            }
            target::SHUFFLE => {
                self.shuffle.handle(event.kind);
                match player.shuffle {
                    Some(on) if clicked => send(Command::Shuffle(!on)),
                    _ => Vec::new(),
                }
            }
            target::REPEAT => {
                self.repeat.handle(event.kind);
                match player.repeat {
                    Some(repeat) if clicked => send(Command::Repeat(repeat.next())),
                    _ => Vec::new(),
                }
            }
            target::SEEK => {
                self.seek.handle(event.kind);
                let fraction =
                    ((event.x - THUMB / 2.0) / (event.width - THUMB).max(1.0)).clamp(0.0, 1.0);
                match (event.kind, player.length) {
                    (PointerKind::Press | PointerKind::Drag, _) => {
                        self.seeking = Some(fraction);
                        Vec::new()
                    }
                    (PointerKind::Release, Some(length)) => {
                        let Some(fraction) = self.seeking.take() else { return Vec::new() };
                        self.sought = Some(fraction);
                        send(Command::Seek(length.mul_f32(fraction)))
                    }
                    // The wheel steps through the track, up for onward.
                    (PointerKind::Scroll(steps), Some(length)) if !length.is_zero() => {
                        let from = match self.sought {
                            Some(fraction) => length.mul_f32(fraction),
                            None => player.position(Instant::now()),
                        };
                        let to = (from.as_secs_f32() - steps * SEEK_STEP)
                            .clamp(0.0, length.as_secs_f32());
                        self.sought = Some(to / length.as_secs_f32());
                        send(Command::Seek(Duration::from_secs_f32(to)))
                    }
                    _ => Vec::new(),
                }
            }
            target::VOLUME => {
                self.volume.handle(event.kind);
                match event.kind {
                    // Dragging only moves the handle; the player is told once, on letting go.
                    PointerKind::Press | PointerKind::Drag => {
                        self.volume_set = Some(Slider::value_at(&event));
                        Vec::new()
                    }
                    PointerKind::Release => match self.volume_set {
                        Some(volume) => self.set_volume(player, volume),
                        None => Vec::new(),
                    },
                    PointerKind::Scroll(steps) => self.nudge_volume(-steps * VOLUME_STEP),
                    _ => Vec::new(),
                }
            }
            id if id >= target::PLAYER => {
                let index = (id - target::PLAYER) as usize;
                if let Some(touch) = self.chips.get_mut(index) {
                    touch.handle(event.kind);
                }
                // A click picks a player; the wheel steps through them.
                let pick = match event.kind {
                    PointerKind::Click => Some(index),
                    PointerKind::Scroll(steps) => {
                        self.tabs_scrolled += steps;
                        let shown = self.players.iter().position(|p| p.bus == player.bus);
                        let last = self.players.len().saturating_sub(1);
                        match shown {
                            Some(shown) if self.tabs_scrolled >= 1.0 => Some((shown + 1).min(last)),
                            Some(shown) if self.tabs_scrolled <= -1.0 => shown.checked_sub(1),
                            _ => None,
                        }
                    }
                    PointerKind::Leave => {
                        self.tabs_scrolled = 0.0;
                        None
                    }
                    _ => None,
                };
                if let Some(picked) = pick.and_then(|pick| self.players.get(pick)) {
                    self.tabs_scrolled = 0.0;
                    if clicked || picked.bus != player.bus {
                        self.pinned = Some(picked.bus.clone());
                        self.choose();
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
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

/// A state layer's opacity: Material's 8% under the pointer and 10% more when pressed.
fn state_layer(touched: Touched) -> f32 {
    0.08 * touched.hover.clamp(0.0, 1.0) + 0.10 * touched.press.clamp(0.0, 1.0)
}

/// A line of type that ends in an ellipsis if there isn't room for it.
fn label(text: &str, size: f32, weight: u16, color: Color) -> Element {
    Element::text(text).ellipsis(true).font_size(size).font_weight(weight).text_color(color)
}

/// An image filling a `size` square with corners rounded by `radius`.
fn art(path: &std::path::Path, size: f32, radius: f32, opacity: f32) -> Element {
    Element::new()
        .size(size, size)
        .shrink(0.0)
        .corner_radius(radius)
        .clip()
        .child(Element::image(Image::new(ImageFile::new(path), size).cover().opacity(opacity)))
}

/// Whether `player`'s volume is its own to set: browsers report one but ignore it.
fn has_volume(player: &Player) -> bool {
    player.rank() < 2
}

/// The last part of a player's D-Bus name that names it: `spotify`, `firefox`.
fn short_name(player: &Player) -> &str {
    let name = player.bus.strip_prefix("org.mpris.MediaPlayer2.").unwrap_or(&player.bus);
    name.split('.').next().unwrap_or(name)
}

/// What to call a player: what it calls itself, tidied, else its D-Bus name.
fn display_name(player: &Player) -> String {
    let name = if player.identity.is_empty() { short_name(player) } else { &player.identity };
    // "Mozilla firefox" → "Firefox".
    let name = name.strip_prefix("Mozilla ").unwrap_or(name);
    let mut letters = name.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// `m:ss`, or `h:mm:ss` from an hour up.
fn clock_time(time: Duration) -> String {
    let seconds = time.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// The card's height without a volume slider, and with one: the tallest it gets.
const CARD_MIN_HEIGHT: f32 = 2.0 * CARD_PADDING
    + TAB_HEIGHT
    + CARD_GAP
    + HERO
    + CARD_GAP
    + SEEK_HEIGHT
    + TIMES_HEIGHT
    + CARD_GAP
    + TRANSPORT_HEIGHT;
const CARD_MAX_HEIGHT: f32 = CARD_MIN_HEIGHT + CARD_GAP + VOLUME_HEIGHT;

/// How tall the card is for `player`.
fn card_height(player: &Player) -> f32 {
    if player.volume.is_some() && has_volume(player) { CARD_MAX_HEIGHT } else { CARD_MIN_HEIGHT }
}

/// The name a player's icon goes by: its desktop entry's, else its D-Bus name's.
fn app_of(player: &Player) -> &str {
    if player.desktop_entry.is_empty() { short_name(player) } else { &player.desktop_entry }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_in_the_font() {
        let names = [
            "music_note",
            "play_arrow",
            "pause",
            "skip_previous",
            "skip_next",
            "shuffle",
            "repeat",
            "repeat_one",
            "volume_mute",
            "volume_down",
            "volume_up",
        ];
        for name in names {
            assert!(shell_m3e::icons::font().contains(name), "{name}");
        }
    }

    #[test]
    fn times_read_as_a_clock_does() {
        assert_eq!(clock_time(Duration::from_secs(5)), "0:05");
        assert_eq!(clock_time(Duration::from_secs(754)), "12:34");
        assert_eq!(clock_time(Duration::from_secs(3723)), "1:02:03");
    }
}
