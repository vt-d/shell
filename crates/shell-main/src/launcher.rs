//! The launcher: a search field in the middle of the screen with the apps under it,
//! opened from outside (`shell launcher_open`, typically bound to a key).
//!
//! It is one sheet. At its top is the field: a badge with the search icon, which turns
//! from a circle into a cookie and a notch further with every keystroke, and what has
//! been typed. Under it is a short list, the apps started most often first until
//! something is typed and the best matches after. The selection is a pill behind a row
//! that stretches toward the row it's moving to before letting go of the one it was on.
//!
//! The sheet drops in a little and unfurls from the field as it opens, its rows coming
//! up one after another, and its height follows the number of matches. It is as
//! see-through as the bar, and the rest of the screen dims behind it: its window covers
//! all of the screen, so a press anywhere off the sheet closes it, and it has the
//! keyboard while it's open.
//!
//! - Typing searches; Backspace, Ctrl+Backspace (or Ctrl+W) and Ctrl+U take it back.
//! - Up and Down (or Tab and Shift+Tab, or Ctrl+P and Ctrl+N) move the selection, Page
//!   Up and Page Down by a list's worth; the wheel scrolls.
//! - Enter or a click starts the app; Escape closes.

use std::{
    f32::consts::TAU,
    time::{Duration, Instant},
};

use shell_m3e::{
    Icon, MaterialShape, Theme, ThemeExt,
    components::Touch,
    motion::{Spring, expressive, standard},
};
use shell_services::apps::{self, App, AppIcons, Launches};
use shell_ui::{
    AlignItems, Color, Element, ExclusiveZone, Extent, Frame, Image, ImageFile, JustifyContent,
    Key, KeyEvent, KeyboardInteractivity, Layer, PointerEvent, PointerKind, Shape, Window,
    WindowId,
};

/// The sheet.
const WIDTH: f32 = 520.0;
const RADIUS: f32 = 32.0;
const PADDING: f32 = 8.0;
/// The width of what's on the sheet.
const INNER: f32 = WIDTH - 2.0 * PADDING;
/// Space between the field and the list.
const GAP: f32 = 8.0;
/// How opaque the sheet is: what's behind shows through the compositor's blur (a layer
/// rule for the `launcher` namespace).
const GLASS: f32 = 0.84;
/// How dark the rest of the screen gets.
const DIM: f32 = 0.22;
/// How much narrower the sheet is as it starts to open, and how far above its place.
const SQUEEZE: f32 = 0.08;
const DROP: f32 = 28.0;
/// Room around the sheet's place to overshoot into as it opens.
const SLACK: f32 = 24.0;

/// The field: as round as the sheet's corners are around it.
const FIELD: f32 = 56.0;
const FIELD_RADIUS: f32 = RADIUS - PADDING;
const BADGE: f32 = 40.0;
const CARET_WIDTH: f32 = 2.0;
const CARET_HEIGHT: f32 = 22.0;
/// How long the caret is shown, and then hidden, while nothing is typed.
const BLINK: u128 = 530;
/// How far the badge turns with a keystroke: one of its cookie's nine lobes.
const NOTCH: f32 = TAU / 9.0;

/// The list: how many rows show at once, and a row.
const ROWS: usize = 5;
const ROW: f32 = 56.0;
const ROW_GAP: f32 = 2.0;
const PITCH: f32 = ROW + ROW_GAP;
const ICON: f32 = 32.0;
/// How far below its place a row starts as it comes up, and how much later each row
/// starts than the one above it, as a share of the sheet's opening.
const RISE: f32 = 14.0;
const STAGGER: f32 = 0.09;
/// How far below their places new matches start.
const SETTLE: f32 = 6.0;

/// The sheet with nothing but the field on it, and with a full list.
const BASE: f32 = FIELD + 2.0 * PADDING;
const HEIGHT_FULL: f32 = BASE + GAP + ROWS as f32 * PITCH - ROW_GAP;

/// What pointer events are on.
mod target {
    pub const SHEET: u32 = 60;
    /// The row of the first match; the others follow.
    pub const ROW: u32 = 1000;
}

pub struct Launcher {
    apps: Vec<App>,
    launches: Launches,
    icons: AppIcons,
    query: String,
    /// The apps that match the query, as indexes into `apps`, best first.
    results: Vec<usize>,
    /// Which of `results` Enter starts.
    selected: usize,
    /// The first of `results` in view.
    first: usize,
    /// What of a wheel's turning hasn't amounted to a row yet.
    wheel: f32,
    is_open: bool,
    /// 0 → 1 as the sheet opens.
    open: Spring,
    window: Option<WindowId>,
    /// The list's height.
    height: Spring,
    /// How far the list is scrolled, in pixels.
    scroll: Spring,
    /// Where the selection's pill is, from the list's top, were it not scrolled: its
    /// edge ahead and its edge behind, which catches up.
    lead: Spring,
    trail: Spring,
    /// 0 → 1 as new matches settle.
    fresh: Spring,
    /// 0 → 1 as something is typed; and how far the badge has turned.
    badge: Spring,
    turn: Spring,
    /// The row under the pointer.
    hover: Option<usize>,
    touch: Touch,
    /// When a key was last pressed: the caret starts blinking again from there.
    typed: Instant,
}

/// How a row is drawn this frame.
struct Look {
    /// Its top, from the list's.
    y: f32,
    opacity: f32,
    /// How much of the selection is on it, 0–1.
    selected: f32,
    /// Its state layer's opacity.
    layer: f32,
}

impl Launcher {
    pub fn new() -> Self {
        Self::with(Vec::new(), Launches::load())
    }

    fn with(apps: Vec<App>, launches: Launches) -> Self {
        let mut launcher = Self {
            apps,
            launches,
            icons: AppIcons::default(),
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            first: 0,
            wheel: 0.0,
            is_open: false,
            open: Spring::new(expressive::SPATIAL, 0.0),
            window: None,
            height: Spring::new(expressive::SPATIAL, 0.0).with_precision(0.01),
            scroll: Spring::new(standard::SPATIAL, 0.0).with_precision(0.01),
            lead: Spring::new(standard::SPATIAL_FAST, 0.0).with_precision(0.01),
            trail: Spring::new(expressive::SPATIAL, 0.0).with_precision(0.01),
            fresh: Spring::new(expressive::SPATIAL_FAST, 1.0),
            badge: Spring::new(expressive::SPATIAL, 0.0),
            turn: Spring::new(expressive::SPATIAL, 0.0),
            hover: None,
            touch: Touch::new(),
            typed: Instant::now(),
        };
        launcher.search();
        launcher
    }

    pub fn set_window(&mut self, window: WindowId) {
        self.window = Some(window);
    }

    /// The launcher's window, while it has one.
    pub fn window(&self) -> Option<WindowId> {
        self.window
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Opens the launcher afresh. Returns whether it needs a window opened for it.
    pub fn open(&mut self) -> bool {
        if !self.is_open {
            self.is_open = true;
            // What's installed may have changed since.
            self.apps = apps::installed();
            self.query.clear();
            self.search();
            self.fresh.snap(1.0);
            self.height.snap(self.list_height());
            self.badge.snap(0.0);
            self.turn.snap(0.0);
            self.hover = None;
            self.touch = Touch::new();
            self.typed = Instant::now();
            // One still closing opens again from where it is.
            self.open = Spring::new(expressive::SPATIAL, self.open.value());
            self.open.set_target(1.0);
        }
        self.window.is_none()
    }

    pub fn close(&mut self) {
        if self.is_open {
            self.is_open = false;
            // Out of the way at once, with no bounce: whatever is typed next is for
            // what's under it.
            self.open = Spring::new(expressive::EFFECTS, self.open.value());
            self.open.set_target(0.0);
        }
    }

    /// Opens the launcher if it's closed and closes it if it's open. Returns whether it
    /// needs a window opened for it.
    pub fn toggle(&mut self) -> bool {
        if self.is_open {
            self.close();
            return false;
        }
        self.open()
    }

    /// Finds the apps that match the query, and selects the best of them.
    fn search(&mut self) {
        let query = self.query.trim().to_lowercase();
        let mut scored: Vec<(u32, usize)> = self
            .apps
            .iter()
            .enumerate()
            .filter_map(|(index, app)| Some((score(app, &query)?, index)))
            .collect();
        // The better match first; of two as good, the one started more often, then the
        // one with less of a name to match, then by name (as the apps are sorted).
        scored.sort_by_key(|&(score, index)| {
            let app = &self.apps[index];
            let length = if query.is_empty() { 0 } else { app.name.len() };
            (u32::MAX - score, u32::MAX - self.launches.count(&app.id), length, index)
        });
        let results: Vec<usize> = scored.into_iter().map(|(_, index)| index).collect();
        if results != self.results {
            self.results = results;
            self.fresh.snap(0.0);
            self.fresh.set_target(1.0);
        }
        self.selected = 0;
        self.first = 0;
        self.scroll.set_target(0.0);
        self.lead.snap(0.0);
        self.trail.snap(0.0);
        self.hover = None;
    }

    /// The height of a list of the matches there are; with none, room to say so.
    fn list_height(&self) -> f32 {
        self.results.len().clamp(1, ROWS) as f32 * PITCH - ROW_GAP
    }

    /// The last match that can be first in view.
    fn last_first(&self) -> usize {
        self.results.len().saturating_sub(ROWS)
    }

    /// Selects a match, scrolling it into view.
    fn select(&mut self, index: usize) {
        let Some(last) = self.results.len().checked_sub(1) else { return };
        self.selected = index.min(last);
        self.first = self.first.clamp((self.selected + 1).saturating_sub(ROWS), self.selected);
        self.scroll.set_target(self.first as f32 * PITCH);
    }

    /// Moves the selection by `by` matches, around the ends.
    fn step(&mut self, by: isize) {
        let count = self.results.len() as isize;
        if count > 0 {
            self.select((self.selected as isize + by).rem_euclid(count) as usize);
        }
    }

    /// Scrolls by `by` rows, keeping the selection in view.
    fn scroll_by(&mut self, by: isize) {
        self.first = (self.first as isize + by).clamp(0, self.last_first() as isize) as usize;
        self.scroll.set_target(self.first as f32 * PITCH);
        let in_view = self.selected.clamp(self.first, self.first + ROWS - 1);
        self.select(in_view);
    }

    /// Starts a match and closes.
    fn launch(&mut self, index: usize) {
        let Some(app) = self.results.get(index).map(|&app| &self.apps[app]) else { return };
        match apps::launch(app) {
            Ok(()) => self.launches.record(&app.id),
            Err(err) => eprintln!("launcher: can't start {}: {err}", app.name),
        }
        self.close();
    }

    /// Takes in a change to the query, the badge turning with it.
    fn edited(&mut self, notches: f32) {
        self.turn.set_target(self.turn.target() + notches * NOTCH);
        self.search();
    }

    /// Takes in a key pressed while the launcher has the keyboard.
    pub fn key_event(&mut self, event: KeyEvent) {
        if !self.is_open {
            return;
        }
        self.typed = Instant::now();
        match event.key {
            Key::Escape => self.close(),
            Key::Enter => self.launch(self.selected),
            Key::Down => self.step(1),
            Key::Up => self.step(-1),
            Key::Tab => self.step(if event.shift { -1 } else { 1 }),
            Key::Char('n' | 'j') if event.ctrl => self.step(1),
            Key::Char('p' | 'k') if event.ctrl => self.step(-1),
            Key::PageDown => self.select(self.selected + ROWS),
            Key::PageUp => self.select(self.selected.saturating_sub(ROWS)),
            Key::Char('u') if event.ctrl => {
                if !self.query.is_empty() {
                    self.query.clear();
                    self.edited(-1.0);
                }
            }
            Key::Backspace | Key::Char('w') if event.ctrl => {
                // The last word, and the space after it.
                let kept = self.query.trim_end();
                let kept = kept.rfind(char::is_whitespace).map_or(0, |space| space + 1);
                if kept < self.query.len() {
                    self.query.truncate(kept);
                    self.edited(-1.0);
                }
            }
            Key::Backspace => {
                if self.query.pop().is_some() {
                    self.edited(-1.0);
                }
            }
            _ => {
                if let Some(text) = event.text {
                    self.query.push_str(&text);
                    self.edited(1.0);
                }
            }
        }
    }

    /// Takes in what the pointer did on the launcher's window.
    pub fn pointer_event(&mut self, event: PointerEvent) {
        if !self.is_open {
            return;
        }
        let Some(id) = event.target else {
            // A press off the sheet.
            if event.kind == PointerKind::Press {
                self.close();
            }
            return;
        };
        if let PointerKind::Scroll(steps) = event.kind {
            self.wheel += steps;
            let rows = self.wheel.trunc();
            self.wheel -= rows;
            self.scroll_by(rows as isize);
            return;
        }
        let Some(index) = id.checked_sub(target::ROW).map(|row| row as usize) else { return };
        self.touch.handle(event.kind);
        match event.kind {
            PointerKind::Enter => self.hover = Some(index),
            PointerKind::Leave => {
                self.hover.take_if(|hover| *hover == index);
            }
            PointerKind::Press => self.select(index),
            PointerKind::Click => self.launch(index),
            _ => {}
        }
    }

    /// The launcher's window: all of the screen, the sheet in the middle of it.
    pub fn view(&mut self, frame: &mut Frame) -> Window {
        let theme = frame.theme();
        let open = frame.animate(&mut self.open);
        if !self.is_open && !self.open.is_moving() {
            frame.close();
            self.window = None;
        }
        // The sheet's top stays where it is whatever the list's length: it hangs from the
        // top of the room a full one takes, which is what's centered.
        let stage = Element::column()
            .align_items(AlignItems::CENTER)
            .size(WIDTH + 2.0 * SLACK, HEIGHT_FULL + SLACK)
            .shrink(0.0)
            .child(self.sheet(frame, theme, open));
        Window::layer_shell("launcher")
            .layer(Layer::Overlay)
            .width(Extent::Fill)
            .height(Extent::Fill)
            // Over the bar too.
            .exclusive_zone(ExclusiveZone::Ignore)
            .keyboard(KeyboardInteractivity::Exclusive)
            .background(Color::BLACK.with_alpha(DIM * open.clamp(0.0, 1.0)))
            .root(Element::column().fill().center().child(stage))
    }

    /// The sheet, `open` (0–1, overshooting) of the way open.
    fn sheet(&mut self, frame: &mut Frame, theme: &Theme, open: f32) -> Element {
        let shown = (open * 3.0).clamp(0.0, 1.0);
        self.height.set_target(self.list_height());
        let list_height = frame.animate(&mut self.height).max(0.0);
        let width = WIDTH * (1.0 - SQUEEZE * (1.0 - open));
        let height = (BASE + (GAP + list_height) * open).max(0.0);

        let content = Element::column()
            // Laid out at the sheet's full width, however wide it is yet.
            .absolute((width - WIDTH) / 2.0, 0.0)
            .width(WIDTH)
            .padding(PADDING)
            .gap(GAP)
            .child(self.field(frame, theme, shown))
            .child(self.list(frame, theme, list_height, open, shown));
        Element::new()
            .size(width, height)
            .shrink(0.0)
            .offset(0.0, -DROP * (1.0 - open))
            .background(theme.surface_container.with_alpha(GLASS * shown))
            .corner_radius(RADIUS.min(height / 2.0))
            .clip()
            .target(target::SHEET)
            .child(content)
    }

    /// The search field: the badge, and what's typed with the caret after it.
    fn field(&mut self, frame: &mut Frame, theme: &Theme, shown: f32) -> Element {
        let typing = !self.query.is_empty();
        self.badge.set_target(if typing { 1.0 } else { 0.0 });
        let morph = frame.animate(&mut self.badge);
        let turn = frame.animate(&mut self.turn);
        let amount = morph.clamp(0.0, 1.0);
        let badge = Element::row()
            .center()
            .size(BADGE, BADGE)
            .shrink(0.0)
            .shape(
                Shape::morph(MaterialShape::Circle, MaterialShape::Cookie9Sided, morph)
                    .rotated(turn),
            )
            .background(fade(theme.secondary_container.lerp(theme.primary, amount), shown))
            .text_color(fade(theme.on_secondary_container.lerp(theme.on_primary, amount), shown))
            .child(Element::symbol(Icon::new("search").size(22.0).weight(500.0)));

        // Steady while typing, blinking after.
        let since = frame.now().saturating_duration_since(self.typed).as_millis();
        let lit = since % (2 * BLINK) < BLINK;
        if self.is_open {
            frame.request_frame_after(Duration::from_millis((BLINK - since % BLINK) as u64 + 1));
        }
        let caret = Element::new()
            .size(CARET_WIDTH, CARET_HEIGHT)
            .shrink(0.0)
            .background(fade(theme.primary, if lit { shown } else { 0.0 }))
            .corner_radius(CARET_WIDTH / 2.0);
        let words = Element::row().align_items(AlignItems::CENTER).grow(1.0).width(0.0).clip();
        let words = if typing {
            // Text ends at its last letter, and the caret belongs after any spaces.
            let typed = self.query.replace(' ', "\u{a0}");
            let typed = Element::text(typed)
                .wrap(false)
                .font_size(18.0)
                .font_weight(450)
                .text_color(fade(theme.on_surface, shown));
            words.gap(1.0).child(typed).child(caret)
        } else {
            let hint = fade(theme.on_surface_variant, shown);
            words.gap(2.0).child(caret).child(label("Search apps", 18.0, 400, hint))
        };
        Element::row()
            .align_items(AlignItems::CENTER)
            .height(FIELD)
            .shrink(0.0)
            .padding_sides(0.0, 20.0, 0.0, (FIELD - BADGE) / 2.0)
            .gap(14.0)
            .background(fade(theme.surface_container_highest, shown))
            .corner_radius(FIELD_RADIUS)
            .child(badge)
            .child(words)
    }

    /// The matches in view, the selection's pill behind them.
    fn list(
        &mut self,
        frame: &mut Frame,
        theme: &Theme,
        height: f32,
        open: f32,
        shown: f32,
    ) -> Element {
        let scroll = frame.animate(&mut self.scroll);
        let place = self.selected as f32 * PITCH;
        self.lead.set_target(place);
        self.trail.set_target(place);
        let lead = frame.animate(&mut self.lead);
        let trail = frame.animate(&mut self.trail);
        let fresh = frame.animate(&mut self.fresh);
        let touched = self.touch.animate(frame);

        let list =
            Element::new().size(INNER, height).shrink(0.0).corner_radius(FIELD_RADIUS).clip();
        if self.results.is_empty() {
            let color = fade(theme.on_surface_variant, shown * fresh.clamp(0.0, 1.0));
            return list.child(
                Element::row()
                    .center()
                    .size(INNER, ROW)
                    .gap(10.0)
                    .text_color(color)
                    .child(Element::symbol(Icon::new("search_off").size(20.0)))
                    .child(label("No apps match", 15.0, 450, color)),
            );
        }

        // The pill reaches from wherever its slower edge still is to where its faster
        // one has got to.
        let (top, bottom) = (lead.min(trail), lead.max(trail) + ROW);
        let mut list = list.child(
            Element::new()
                .absolute(0.0, top - scroll)
                .size(INNER, bottom - top)
                .background(fade(theme.secondary_container, shown))
                .corner_radius(FIELD_RADIUS),
        );
        let from = ((scroll / PITCH).floor() as isize - 1).max(0) as usize;
        let to = (from + ROWS + 3).min(self.results.len());
        let middle = (top + bottom - ROW) / 2.0;
        for index in from..to {
            let y = index as f32 * PITCH - scroll;
            // Each row sets off a little after the one above it.
            let delay = STAGGER * (y / PITCH).clamp(0.0, ROWS as f32);
            let risen = ((open - delay) / (1.0 - delay)).min(1.2);
            let look = Look {
                y: y + RISE * (1.0 - risen) + SETTLE * (1.0 - fresh),
                opacity: shown * risen.clamp(0.0, 1.0) * (0.4 + 0.6 * fresh.clamp(0.0, 1.0)),
                selected: (1.0 - (middle - index as f32 * PITCH).abs() / PITCH).clamp(0.0, 1.0),
                layer: if self.hover == Some(index) {
                    0.08 * touched.hover.clamp(0.0, 1.0) + 0.10 * touched.press.clamp(0.0, 1.0)
                } else {
                    0.0
                },
            };
            list = list.child(self.row(theme, index, look));
        }
        list
    }

    /// A match: its icon, its name over what it is, and on the selected one the key that
    /// starts it.
    fn row(&mut self, theme: &Theme, index: usize, look: Look) -> Element {
        let app = &self.apps[self.results[index]];
        let file = app.icon.as_deref().and_then(|name| self.icons.named(name)).map(ImageFile::new);
        let icon = match file {
            Some(file) => Element::image(Image::new(file, ICON).opacity(look.opacity)),
            // No icon: its initial.
            None => {
                let initial: String =
                    app.name.chars().next().into_iter().flat_map(char::to_uppercase).collect();
                let color = fade(theme.on_tertiary_container, look.opacity);
                Element::row()
                    .center()
                    .size(ICON, ICON)
                    .shrink(0.0)
                    .background(fade(theme.tertiary_container, look.opacity))
                    .corner_radius(ICON / 2.0)
                    .child(label(&initial, 15.0, 700, color))
            }
        };
        let ink = theme.on_surface.lerp(theme.on_secondary_container, look.selected);
        let quiet = theme.on_surface_variant.lerp(theme.on_secondary_container, look.selected);
        let mut words = Element::column()
            .grow(1.0)
            .width(0.0)
            .justify_content(JustifyContent::CENTER)
            .child(label(&app.name, 15.0, 500, fade(ink, look.opacity)));
        if let Some(description) = &app.description {
            words = words.child(label(description, 12.5, 400, fade(quiet, look.opacity * 0.9)));
        }
        let mut row = Element::row()
            .absolute(0.0, look.y)
            .size(INNER, ROW)
            .align_items(AlignItems::CENTER)
            .padding_sides(0.0, 18.0, 0.0, 14.0)
            .gap(14.0)
            .target(target::ROW + index as u32)
            .child(
                Element::new()
                    .absolute(0.0, 0.0)
                    .size(INNER, ROW)
                    .background(fade(theme.on_surface, look.layer))
                    .corner_radius(FIELD_RADIUS),
            )
            .child(icon)
            .child(words);
        if look.selected > 0.0 {
            let hint = Icon::new("keyboard_return").size(18.0);
            row = row.child(
                Element::symbol(hint)
                    .text_color(fade(quiet, look.opacity * look.selected * 0.8))
                    .offset(6.0 * (1.0 - look.selected), 0.0),
            );
        }
        row
    }
}

/// How well `app` matches `query` (in lower case), if it does at all: its name first,
/// from its start, then the start of a word in it, its initials, anywhere in it; then
/// what it's filed under and described as; then its name with letters left out.
fn score(app: &App, query: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let name = app.name.to_lowercase();
    let words = || name.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty());
    let initials: String = words().filter_map(|word| word.chars().next()).collect();
    let id = app.id.rsplit('.').next().unwrap_or_default().to_lowercase();
    let description = app.description.as_deref().unwrap_or_default().to_lowercase();
    let score = if name == query {
        100
    } else if name.starts_with(query) {
        90
    } else if words().any(|word| word.starts_with(query)) {
        80
    } else if query.chars().count() > 1 && initials.starts_with(query) {
        70
    } else if name.contains(query) {
        60
    } else if id.starts_with(query) {
        50
    } else if app.keywords.iter().any(|keyword| keyword.starts_with(query)) {
        40
    } else if description.split_whitespace().any(|word| word.starts_with(query)) {
        30
    } else if id.contains(query) || description.contains(query) {
        20
    } else if in_order(&name, query) {
        10
    } else {
        return None;
    };
    Some(score)
}

/// Whether `text` has the letters of `query`, in that order, among its own.
fn in_order(text: &str, query: &str) -> bool {
    let mut letters = text.chars();
    query.chars().filter(|c| !c.is_whitespace()).all(|c| letters.any(|letter| letter == c))
}

/// `color`, `opacity` as opaque as it was.
fn fade(color: Color, opacity: f32) -> Color {
    color.with_alpha(color.a * opacity)
}

/// A line of type that ends in an ellipsis if there isn't room for it.
fn label(text: &str, size: f32, weight: u16, color: Color) -> Element {
    Element::text(text).ellipsis(true).font_size(size).font_weight(weight).text_color(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str, description: &str, keywords: &[&str]) -> App {
        App {
            id: id.to_owned(),
            name: name.to_owned(),
            description: Some(description.to_owned()).filter(|d| !d.is_empty()),
            keywords: keywords.iter().map(|&keyword| keyword.to_owned()).collect(),
            icon: None,
            command: id.to_owned(),
            terminal: false,
        }
    }

    fn launcher() -> Launcher {
        let apps = vec![
            app("org.gnome.Calculator", "Calculator", "", &["math"]),
            app("org.gnome.Nautilus", "Files", "File Manager", &["folder"]),
            app("firefox", "Firefox", "Web Browser", &["internet"]),
            app("kitty", "kitty", "Terminal Emulator", &["shell"]),
            app("code", "Visual Studio Code", "Text Editor", &[]),
            app("org.pulseaudio.pavucontrol", "Volume Control", "", &["audio"]),
        ];
        let mut launcher = Launcher::with(apps, Launches::default());
        launcher.is_open = true;
        launcher
    }

    fn names(launcher: &Launcher) -> Vec<&str> {
        launcher.results.iter().map(|&index| launcher.apps[index].name.as_str()).collect()
    }

    fn key(key: Key) -> KeyEvent {
        KeyEvent { key, text: None, ctrl: false, alt: false, shift: false }
    }

    fn typed(launcher: &mut Launcher, text: &str) {
        for c in text.chars() {
            launcher.key_event(KeyEvent { text: Some(c.to_string()), ..key(Key::Char(c)) });
        }
    }

    #[test]
    fn every_icon_is_in_the_font() {
        for name in ["search", "search_off", "keyboard_return"] {
            assert!(shell_m3e::icons::font().contains(name), "{name}");
        }
    }

    #[test]
    fn lists_everything_by_name_until_something_is_typed() {
        let launcher = launcher();
        assert_eq!(
            names(&launcher),
            ["Calculator", "Files", "Firefox", "kitty", "Visual Studio Code", "Volume Control"]
        );
    }

    #[test]
    fn the_better_match_comes_first() {
        let mut launcher = launcher();
        typed(&mut launcher, "fi");
        // From the start of the name, the shorter first.
        assert_eq!(names(&launcher), ["Files", "Firefox"]);
        launcher.key_event(KeyEvent { ctrl: true, ..key(Key::Char('u')) });
        typed(&mut launcher, "co");
        // A word's start, the shorter name first; then letters in order.
        assert_eq!(names(&launcher), ["Volume Control", "Visual Studio Code", "Calculator"]);
        launcher.key_event(KeyEvent { ctrl: true, ..key(Key::Char('u')) });
        typed(&mut launcher, "vsc");
        assert_eq!(names(&launcher), ["Visual Studio Code"]);
        launcher.key_event(KeyEvent { ctrl: true, ..key(Key::Char('u')) });
        typed(&mut launcher, "Terminal");
        assert_eq!(names(&launcher), ["kitty"]);
        typed(&mut launcher, "zzz");
        assert!(launcher.results.is_empty());
    }

    #[test]
    fn keys_edit_the_query() {
        let mut launcher = launcher();
        typed(&mut launcher, "web brow");
        assert_eq!(launcher.query, "web brow");
        launcher.key_event(key(Key::Backspace));
        assert_eq!(launcher.query, "web bro");
        launcher.key_event(KeyEvent { ctrl: true, ..key(Key::Backspace) });
        assert_eq!(launcher.query, "web ");
        launcher.key_event(KeyEvent { ctrl: true, ..key(Key::Char('w')) });
        assert_eq!(launcher.query, "");
        // A shortcut types nothing.
        launcher.key_event(KeyEvent { ctrl: true, ..key(Key::Char('a')) });
        assert_eq!(launcher.query, "");
    }

    #[test]
    fn the_selection_goes_around_and_stays_in_view() {
        let mut launcher = launcher();
        launcher.key_event(key(Key::Up));
        assert_eq!((launcher.selected, launcher.first), (5, 1));
        launcher.key_event(key(Key::Down));
        assert_eq!((launcher.selected, launcher.first), (0, 0));
        launcher.key_event(key(Key::PageDown));
        assert_eq!((launcher.selected, launcher.first), (5, 1));
        launcher.scroll_by(-1);
        assert_eq!((launcher.selected, launcher.first), (4, 0));
        // Typing starts again from the top.
        typed(&mut launcher, "f");
        assert_eq!((launcher.selected, launcher.first), (0, 0));
    }

    #[test]
    fn escape_and_a_press_off_the_sheet_close_it() {
        let mut launcher = launcher();
        launcher.key_event(key(Key::Escape));
        assert!(!launcher.is_open());
        launcher.is_open = true;
        launcher.pointer_event(PointerEvent {
            target: None,
            kind: PointerKind::Press,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        });
        assert!(!launcher.is_open());
    }
}
