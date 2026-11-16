//! The system tray: a drawer of the icons apps keep there, at the head of the status
//! group.
//!
//! It takes no room until something is in it, and then only a small segment's, which
//! becomes the group's first: a chevron. A click on that draws the segment out to hold
//! the icons: the chevron turns about, the segment takes an accent tint, and the icons come
//! out one after another from it onward, each landing with a bounce. Another click
//! folds them away. An icon swells a little under the pointer. An item that wants
//! attention gets a dot, and so does the chevron while that item is folded away.
//!
//! - A click does what the item does (usually, shows or hides its app's window).
//! - A right click opens its menu, as does a click on an item that is only a menu.
//! - A middle click and the wheel are passed on to it.
//!
//! The menu drops from the bar under the pointer: the item's name, then its entries.
//! One that opens a menu of its own replaces what's shown with it, a row at the top
//! leading back. Like the other things the bar opens it is two windows: itself, and a
//! [scrim](Tray::scrim) under it that takes the press that closes it.

use shell_m3e::{
    Icon, Theme, ThemeExt,
    components::{Touch, Touched},
    motion::{Spring, expressive},
};
use shell_services::tray::{Item, MenuEntry};
use shell_ui::{
    AlignItems, Anchor, Color, Element, Extent, Frame, Image, ImageFile, Layer, Margin,
    PointerEvent, PointerKind, Window, WindowId,
};

use crate::status::{HEIGHT, INNER_RADIUS, SEAM, pill_fill};

/// An item's place in the pill, and the icon in it.
const SLOT: f32 = 32.0;
const ICON: f32 = 18.0;
/// Space between the drawer's ends and the items at them.
const DRAWER_PADDING: f32 = 6.0;
/// How much later each item sets off than the one before it, as a share of the drawer's
/// opening; and how far toward the toggle it starts.
const STAGGER: f32 = 0.12;
const SLIDE: f32 = 10.0;
/// How much an icon swells under the pointer, and gives under a press.
const HOVER_SWELL: f32 = 0.16;
const PRESS_GIVE: f32 = 0.12;
/// The toggle's chevron, and how far it travels as it turns about.
const CHEVRON: f32 = 20.0;
const TURN: f32 = 8.0;
/// How far toward the seam the chevron sits: the segment is round on its other side
/// only, and its middle looks to be there.
const NUDGE: f32 = 1.5;
/// How far the segment's fill goes toward the accent container while the drawer is out.
const TINT: f32 = 0.85;
/// The toggle's width.
const TOGGLE: f32 = 42.0;
/// The dot on an item that wants attention.
const DOT: f32 = 7.0;

/// The menu.
const MENU_WIDTH: f32 = 272.0;
const MENU_RADIUS: f32 = 20.0;
const MENU_PADDING: f32 = 8.0;
/// The item's name, at the top; an entry; and a line between groups of them.
const HEADER: f32 = 40.0;
const HEADER_ICON: f32 = 18.0;
const ROW: f32 = 40.0;
const SEPARATOR: f32 = 9.0;
const ROW_RADIUS: f32 = MENU_RADIUS - MENU_PADDING;
/// How opaque the menu is, as the other panels are.
const GLASS: f32 = 0.84;
/// Space between the bar and the menu.
const MARGIN: f32 = 8.0;
/// Room in the menu's window to either side of it, to overshoot into as it opens.
const SLACK: f32 = 16.0;
/// The tallest a menu gets; more of it than this is cut off.
const MENU_MAX: f32 = 640.0;
/// The nearest the menu comes to the screen's edge.
const EDGE: f32 = 8.0;

/// What pointer events are on: on the bar, and in the menu.
mod target {
    pub const CHEVRON: u32 = 3000;
    /// The first item; the others follow.
    pub const ITEM: u32 = 3001;

    pub const MENU: u32 = 70;
    pub const BACK: u32 = 71;
    /// The first entry in view; the others follow.
    pub const ENTRY: u32 = 100;
}

/// What the tray asks for in answer to the pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// What the item with this key does when clicked, and when middle-clicked.
    Activate(String),
    SecondaryActivate(String),
    /// Have it show a menu of its own making.
    ContextMenu(String),
    /// Turn the wheel over it by this many steps.
    Scroll(String, i32),
    /// Ask for its menu, to be [shown](Tray::show_menu) when it comes.
    RequestMenu(String),
    /// Choose the entry with this id from its menu.
    Choose(String, i32),
    /// Open a window for the [scrim](Tray::scrim), and for the [menu](Tray::menu).
    OpenScrim,
    OpenMenu,
}

pub struct Tray {
    entries: Vec<Entry>,
    /// Whether the drawer is out; 0 → 1 as it comes.
    expanded: bool,
    spread: Spring,
    chevron: Touch,
    /// How much of the tray is there, 0–1, as last drawn.
    presence: f32,
    /// What of a wheel's turning over an item hasn't amounted to a step yet.
    wheel: f32,

    menu: Option<Menu>,
    menu_open: bool,
    /// 0 → 1 as the menu opens.
    open: Spring,
    /// The height of what the menu shows.
    height: Spring,
    /// How far the menu's right edge is from the screen's.
    right: f32,
    menu_window: Option<WindowId>,
    scrim_window: Option<WindowId>,
    /// The menu's row under the pointer, by its target.
    hover: Option<u32>,
    touch: Touch,
}

/// An item on the bar, animated.
struct Entry {
    item: Item,
    touch: Touch,
    /// 0 → 1 as it arrives, and back as it goes.
    shown: Spring,
    /// Its app took it away: it's closing up, and then is dropped.
    gone: bool,
}

/// An item's menu, and where in it the reader is.
struct Menu {
    key: String,
    title: String,
    /// The item's icon, as on the bar.
    icon: Option<(ImageFile, bool)>,
    entries: Vec<MenuEntry>,
    /// The entries opened to get to the menu shown, from the top.
    path: Vec<usize>,
}

impl Menu {
    /// The entry whose menu is shown, if it isn't the top one; and the entries shown.
    fn level(&self) -> (Option<&MenuEntry>, &[MenuEntry]) {
        let mut parent = None;
        let mut entries = &self.entries[..];
        for &index in &self.path {
            let Some(entry) = entries.get(index) else { break };
            parent = Some(entry);
            entries = &entry.children;
        }
        (parent, entries)
    }

    /// How tall what's shown is.
    fn height(&self) -> f32 {
        let (parent, entries) = self.level();
        let top = if parent.is_some() { ROW } else { HEADER };
        let rows: f32 =
            entries.iter().map(|entry| if entry.separator { SEPARATOR } else { ROW }).sum();
        (2.0 * MENU_PADDING + top + rows).min(MENU_MAX)
    }
}

impl Tray {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            expanded: false,
            spread: Spring::new(expressive::SPATIAL, 0.0),
            chevron: Touch::new(),
            presence: 0.0,
            wheel: 0.0,
            menu: None,
            menu_open: false,
            open: Spring::new(expressive::SPATIAL, 0.0),
            height: Spring::new(expressive::SPATIAL, 0.0).with_precision(0.01),
            right: EDGE,
            menu_window: None,
            scrim_window: None,
            hover: None,
            touch: Touch::new(),
        }
    }

    /// Takes in the items there are now.
    pub fn set_items(&mut self, items: Vec<Item>) {
        for entry in &mut self.entries {
            if !items.iter().any(|item| item.key == entry.item.key) {
                entry.gone = true;
                entry.shown.set_target(0.0);
            }
        }
        for item in items {
            match self.entries.iter_mut().find(|entry| entry.item.key == item.key) {
                Some(entry) => {
                    entry.item = item;
                    entry.gone = false;
                    entry.shown.set_target(1.0);
                }
                None => {
                    let mut shown = Spring::new(expressive::SPATIAL, 0.0);
                    shown.set_target(1.0);
                    self.entries.push(Entry { item, touch: Touch::new(), shown, gone: false });
                }
            }
        }
        // A menu doesn't outlast its item.
        let open = self.menu.as_ref().map(|menu| menu.key.as_str());
        if open.is_some_and(|key| !self.entries.iter().any(|e| !e.gone && e.item.key == key)) {
            self.close_menu();
        }
    }

    /// The toggle and the drawer, as the segments that lead the status group; nothing
    /// at all while the tray is empty.
    pub fn view(&mut self, frame: &mut Frame) -> Element {
        let theme = frame.theme();
        let shown: Vec<f32> =
            self.entries.iter_mut().map(|entry| frame.animate(&mut entry.shown)).collect();
        // What has closed up is gone.
        let mut index = 0;
        let shown: Vec<f32> = shown
            .into_iter()
            .filter(|_| {
                let entry = &self.entries[index];
                let keep = !(entry.gone && !entry.shown.is_moving());
                if keep {
                    index += 1;
                } else {
                    self.entries.remove(index);
                }
                keep
            })
            .map(|amount| amount.max(0.0))
            .collect();
        if self.entries.iter().all(|entry| entry.gone) {
            self.expanded = false;
        }
        self.spread.set_target(if self.expanded { 1.0 } else { 0.0 });
        let spread = frame.animate(&mut self.spread);
        self.presence = shown.iter().fold(0.0f32, |most, &amount| most.max(amount)).min(1.0);
        if self.entries.is_empty() {
            return Element::new();
        }
        let presence = self.presence;
        let open = spread.clamp(0.0, 1.0);

        // One segment, the group's first: round at the group's end and nearly square
        // where it meets the next. The toggle is its leading end, a chevron that turns
        // about while the drawer is out; the segment grows to hold the items after it and
        // takes an accent tint.
        let touched = self.chevron.animate(frame);
        let press = touched.press.clamp(0.0, 1.0);
        let ink = theme.on_secondary_container.lerp(theme.on_primary_container, open);
        let chevron = |name: &'static str, shown: f32, slide: f32| {
            Element::symbol(Icon::new(name).size(CHEVRON).weight(600.0))
                .absolute((TOGGLE - CHEVRON) / 2.0 + NUDGE, (HEIGHT - CHEVRON) / 2.0)
                .text_color(fade(ink, shown * presence))
                .offset(slide, 0.0)
        };
        // The state layer is a circle about the chevron, which gives under a press.
        let layer = SLOT * (1.0 - PRESS_GIVE * press);
        let urgent = self.entries.iter().any(|entry| !entry.gone && entry.item.urgent);
        let mut toggle = Element::new()
            .size(TOGGLE * presence, HEIGHT)
            .shrink(0.0)
            .clip()
            .target(target::CHEVRON)
            .child(
                Element::new()
                    .absolute((TOGGLE - layer) / 2.0 + NUDGE, (HEIGHT - layer) / 2.0)
                    .size(layer, layer)
                    .background(fade(ink, state_layer(touched)))
                    .corner_radius(layer / 2.0),
            )
            .child(chevron("chevron_left", 1.0 - open, -TURN * open))
            .child(chevron("chevron_right", open, TURN * (1.0 - open)));
        if urgent {
            // What's folded away wants attention.
            toggle = toggle.child(
                Element::new()
                    .absolute(TOGGLE - DOT - 6.0, 7.0)
                    .size(DOT, DOT)
                    .background(fade(theme.error, (1.0 - open) * presence))
                    .corner_radius(DOT / 2.0),
            );
        }

        // The items, the first to come nearest the rest of the group. Each sets off a
        // little after the one before it, from the toggle onward, and lands with a
        // bounce.
        let count = self.entries.len();
        let mut segment = Element::row()
            .align_items(AlignItems::CENTER)
            .height(HEIGHT)
            .shrink(0.0)
            .padding_sides(0.0, DRAWER_PADDING * open, 0.0, 0.0)
            // Tinted while it is out: open, without a change of shape.
            .background(pill_fill(theme).lerp(theme.primary_container, TINT * open))
            .corner_radii(HEIGHT / 2.0, INNER_RADIUS, INNER_RADIUS, HEIGHT / 2.0)
            .clip()
            .child(toggle);
        for index in (0..count).rev() {
            let entry = &mut self.entries[index];
            let delay = (STAGGER * (count - 1 - index) as f32).min(0.5);
            let risen = ((spread - delay) / (1.0 - delay)).max(0.0);
            let amount = shown[index] * risen;
            let touched = entry.touch.animate(frame);
            let opacity = amount.clamp(0.0, 1.0);
            // It swells a little under the pointer and gives under a press.
            let lift = 1.0 + HOVER_SWELL * touched.hover - PRESS_GIVE * touched.press;
            let size = ICON * (0.4 + 0.6 * amount.min(1.25)) * lift;
            let icon = match &entry.item.icon {
                Some(path) => {
                    let image = Image::new(ImageFile::new(path), size).opacity(opacity);
                    Element::image(if entry.item.symbolic { image.tinted() } else { image })
                }
                // No icon: its initial.
                None => {
                    let ink = fade(theme.on_tertiary_container, opacity);
                    Element::row()
                        .center()
                        .size(size + 4.0, size + 4.0)
                        .shrink(0.0)
                        .background(fade(theme.tertiary_container, opacity))
                        .corner_radius(size)
                        .child(label(&initial(&entry.item.title), 11.0, 700, ink))
                }
            };
            let mut item = slot(SLOT * amount.min(1.1), theme, touched)
                .text_color(fade(theme.on_secondary_container, opacity))
                .child(icon.offset(-SLIDE * (1.0 - risen.min(1.0)), 0.0));
            if !entry.gone && amount > 0.5 {
                item = item.target(target::ITEM + index as u32);
            }
            if entry.item.urgent {
                item = item.child(
                    Element::new()
                        .absolute(SLOT * amount - DOT - 3.0, 4.0)
                        .size(DOT, DOT)
                        .background(fade(theme.error, opacity))
                        .corner_radius(DOT / 2.0),
                );
            }
            segment = segment.child(item);
        }
        // A seam after it, as between the group's other segments.
        Element::row()
            .align_items(AlignItems::CENTER)
            .child(segment)
            .child(Element::new().width(SEAM * presence).shrink(0.0))
    }

    /// How much of the tray is there, 0–1, as of when it was last drawn: how far the
    /// segment after it is from being the group's first.
    pub fn presence(&self) -> f32 {
        self.presence
    }

    /// Takes in what the pointer did on the bar.
    pub fn pointer_event(&mut self, event: PointerEvent) -> Vec<Action> {
        let Some(id) = event.target else { return Vec::new() };
        if id == target::CHEVRON {
            self.chevron.handle(event.kind);
            if event.kind == PointerKind::Click {
                self.expanded = !self.expanded;
                // A menu doesn't stay behind when its item is folded away.
                self.close_menu();
            }
            return Vec::new();
        }
        let entry =
            id.checked_sub(target::ITEM).and_then(|index| self.entries.get_mut(index as usize));
        let Some(entry) = entry.filter(|entry| !entry.gone) else { return Vec::new() };
        entry.touch.handle(event.kind);
        let key = entry.item.key.clone();
        let menu: fn(String) -> Action =
            if entry.item.has_menu { Action::RequestMenu } else { Action::ContextMenu };
        match event.kind {
            PointerKind::Click if entry.item.menu_only => vec![menu(key)],
            PointerKind::Click => vec![Action::Activate(key)],
            PointerKind::SecondaryPress => vec![menu(key)],
            PointerKind::MiddlePress => vec![Action::SecondaryActivate(key)],
            PointerKind::Scroll(steps) => {
                self.wheel += steps;
                let whole = self.wheel.trunc();
                self.wheel -= whole;
                if whole == 0.0 { Vec::new() } else { vec![Action::Scroll(key, whole as i32)] }
            }
            _ => Vec::new(),
        }
    }

    /// Whether the menu shown is that of the item with this key.
    pub fn menu_is_open_for(&self, key: &str) -> bool {
        self.menu_open && self.menu.as_ref().is_some_and(|menu| menu.key == key)
    }

    pub fn menu_is_open(&self) -> bool {
        self.menu_open
    }

    /// Shows the menu of the item with this key, now that it has come, its middle
    /// `center` from the screen's right edge on a screen `width` wide.
    pub fn show_menu(
        &mut self,
        key: String,
        entries: Vec<MenuEntry>,
        center: f32,
        width: f32,
    ) -> Vec<Action> {
        let Some(entry) = self.entries.iter().find(|entry| !entry.gone && entry.item.key == key)
        else {
            return Vec::new();
        };
        if entries.is_empty() {
            return Vec::new();
        }
        let room = (width - MENU_WIDTH - EDGE).max(EDGE);
        self.right = (center - MENU_WIDTH / 2.0).clamp(EDGE, room);
        let icon = entry.item.icon.as_ref().map(|path| (ImageFile::new(path), entry.item.symbolic));
        let menu = Menu { key, title: entry.item.title.clone(), icon, entries, path: Vec::new() };
        self.height.snap(menu.height());
        self.menu = Some(menu);
        self.menu_open = true;
        self.open.set_target(1.0);
        self.hover = None;
        self.touch = Touch::new();
        let mut actions = Vec::new();
        if self.scrim_window.is_none() {
            actions.push(Action::OpenScrim);
        }
        if self.menu_window.is_none() {
            actions.push(Action::OpenMenu);
        }
        actions
    }

    pub fn close_menu(&mut self) {
        self.menu_open = false;
        self.open.set_target(0.0);
    }

    pub fn set_menu_window(&mut self, window: WindowId) {
        self.menu_window = Some(window);
    }

    pub fn set_scrim_window(&mut self, window: WindowId) {
        self.scrim_window = Some(window);
    }

    /// The windows open for the menu, to refresh along with the bar.
    pub fn windows(&self) -> impl Iterator<Item = WindowId> + use<> {
        self.menu_window.into_iter().chain(self.scrim_window)
    }

    /// A window over all of the screen under the bar, with nothing in it: a press on it
    /// is a press off the menu, which closes the menu.
    pub fn scrim(&mut self, frame: &mut Frame) -> Window {
        if !self.menu_open {
            frame.close();
            self.scrim_window = None;
        }
        Window::layer_shell("tray-menu-scrim").width(Extent::Fill).height(Extent::Fill)
    }

    /// The menu's window: under the bar, where the click that asked for it was.
    pub fn menu(&mut self, frame: &mut Frame) -> Window {
        let theme = frame.theme();
        let open = frame.animate(&mut self.open);
        if !self.menu_open && !self.open.is_moving() {
            frame.close();
            self.menu_window = None;
            self.menu = None;
        }
        let sheet = self.sheet(frame, theme, open);
        let root = Element::column()
            .align_items(AlignItems::END)
            .padding_sides(MARGIN, SLACK, 0.0, SLACK)
            .child(sheet);
        // Above the scrim, whichever of the two the compositor happens to map first.
        Window::layer_shell("tray-menu")
            .layer(Layer::Overlay)
            .anchor(Anchor::TOP | Anchor::RIGHT)
            .margin(Margin::new(0, (self.right - SLACK) as i32, 0, 0))
            .width(Extent::Fixed((MENU_WIDTH + 2.0 * SLACK) as u32))
            .height(Extent::Fixed((MARGIN + MENU_MAX * 1.1) as u32))
            .root(root)
    }

    /// The menu itself, `open` (0–1, overshooting) of the way out.
    fn sheet(&mut self, frame: &mut Frame, theme: &Theme, open: f32) -> Element {
        let Some(menu) = &self.menu else { return Element::new() };
        self.height.set_target(menu.height());
        let height = (frame.animate(&mut self.height) * open).max(0.0);
        let shown = (open * 3.0).clamp(0.0, 1.0);
        let touched = self.touch.animate(frame);
        let layer = |id: u32| {
            if self.hover == Some(id) { state_layer(touched) } else { 0.0 }
        };
        let (parent, entries) = menu.level();
        let ticks = entries.iter().any(|entry| entry.checked.is_some());

        let ink = fade(theme.on_surface, shown);
        let quiet = fade(theme.on_surface_variant, shown);
        let mut content = Element::column().width(MENU_WIDTH).padding(MENU_PADDING);
        content = content.child(match parent {
            // Back to the menu this one was opened from.
            Some(parent) => row(target::BACK, fade(theme.on_surface, layer(target::BACK)))
                .text_color(ink)
                .child(Element::symbol(Icon::new("arrow_back").size(20.0)))
                .child(label(&parent.label, 14.0, 600, ink)),
            // Whose menu it is.
            None => {
                let mut header = Element::row()
                    .align_items(AlignItems::CENTER)
                    .height(HEADER)
                    .shrink(0.0)
                    .padding_xy(12.0, 0.0)
                    .gap(10.0)
                    .text_color(quiet);
                if let Some((file, symbolic)) = menu.icon {
                    let image = Image::new(file, HEADER_ICON).opacity(shown);
                    header =
                        header.child(Element::image(if symbolic { image.tinted() } else { image }));
                }
                header.child(label(&menu.title, 13.0, 700, quiet))
            }
        });
        for (index, entry) in entries.iter().enumerate() {
            if entry.separator {
                content = content.child(
                    Element::row()
                        .align_items(AlignItems::CENTER)
                        .height(SEPARATOR)
                        .shrink(0.0)
                        .child(
                            Element::new()
                                .grow(1.0)
                                .height(1.0)
                                .background(fade(theme.outline, 0.3 * shown)),
                        ),
                );
                continue;
            }
            let id = target::ENTRY + index as u32;
            let color = if entry.enabled { ink } else { fade(theme.on_surface, 0.38 * shown) };
            let mut item = row(id, fade(theme.on_surface, layer(id))).text_color(color);
            if ticks {
                let ticked = if entry.checked == Some(true) { shown } else { 0.0 };
                let tick = Icon::new("check").size(18.0).weight(600.0);
                item = item.child(Element::symbol(tick).text_color(fade(theme.primary, ticked)));
            }
            item = item.child(label(&entry.label, 14.0, 450, color).grow(1.0).width(0.0));
            if !entry.children.is_empty() {
                item = item.child(
                    Element::symbol(Icon::new("chevron_right").size(20.0)).text_color(quiet),
                );
            }
            content = content.child(item);
        }
        Element::new()
            .size(MENU_WIDTH, height)
            .shrink(0.0)
            // Glass: what's behind shows through the compositor's blur (a layer rule for
            // the `tray-menu` namespace).
            .background(theme.surface_container.with_alpha(GLASS * shown))
            .corner_radius(MENU_RADIUS.min(height / 2.0))
            .clip()
            .target(target::MENU)
            .child(content.absolute(0.0, 0.0))
    }

    /// Takes in what the pointer did on either of the menu's windows.
    pub fn menu_event(&mut self, event: PointerEvent) -> Vec<Action> {
        let Some(id) = event.target else {
            // A press off the menu.
            if matches!(event.kind, PointerKind::Press | PointerKind::SecondaryPress) {
                self.close_menu();
            }
            return Vec::new();
        };
        if id == target::MENU {
            return Vec::new();
        }
        self.touch.handle(event.kind);
        match event.kind {
            PointerKind::Enter => self.hover = Some(id),
            PointerKind::Leave => {
                self.hover.take_if(|hover| *hover == id);
            }
            PointerKind::Click => return self.clicked(id),
            _ => {}
        }
        Vec::new()
    }

    /// Does what a click on the menu's row with this target does.
    fn clicked(&mut self, id: u32) -> Vec<Action> {
        let Some(menu) = &mut self.menu else { return Vec::new() };
        if id == target::BACK {
            menu.path.pop();
            self.hover = None;
            return Vec::new();
        }
        let Some(index) = id.checked_sub(target::ENTRY).map(|index| index as usize) else {
            return Vec::new();
        };
        let (_, entries) = menu.level();
        let Some(entry) = entries.get(index).filter(|entry| entry.enabled && !entry.separator)
        else {
            return Vec::new();
        };
        let (opens, entry) = (!entry.children.is_empty(), entry.id);
        if opens {
            menu.path.push(index);
            self.hover = None;
            return Vec::new();
        }
        let action = Action::Choose(menu.key.clone(), entry);
        self.close_menu();
        vec![action]
    }
}

/// An item's place in the pill, `width` wide: round, with a state layer under the
/// pointer.
fn slot(width: f32, theme: &Theme, touched: Touched) -> Element {
    let side = SLOT.min(width);
    Element::row().center().size(width, SLOT).shrink(0.0).clip().child(
        Element::new()
            .absolute((width - side) / 2.0, (SLOT - side) / 2.0)
            .size(side, side)
            .background(fade(theme.on_secondary_container, state_layer(touched)))
            .corner_radius(side / 2.0),
    )
}

/// A row of the menu that takes the pointer, with its state layer.
fn row(id: u32, layer: Color) -> Element {
    Element::row()
        .align_items(AlignItems::CENTER)
        .height(ROW)
        .shrink(0.0)
        .padding_xy(12.0, 0.0)
        .gap(10.0)
        .target(id)
        .child(
            Element::new()
                .absolute(0.0, 0.0)
                .size(MENU_WIDTH - 2.0 * MENU_PADDING, ROW)
                .background(layer)
                .corner_radius(ROW_RADIUS),
        )
}

/// The first letter of `name`, as a capital.
fn initial(name: &str) -> String {
    name.chars().next().into_iter().flat_map(char::to_uppercase).collect()
}

/// A state layer's opacity.
fn state_layer(touched: Touched) -> f32 {
    0.08 * touched.hover.clamp(0.0, 1.0) + 0.10 * touched.press.clamp(0.0, 1.0)
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

    fn item(key: &str) -> Item {
        Item {
            key: key.to_owned(),
            title: key.to_owned(),
            icon: None,
            symbolic: false,
            urgent: false,
            has_menu: true,
            menu_only: false,
        }
    }

    fn entry(id: i32, label: &str, children: Vec<MenuEntry>) -> MenuEntry {
        MenuEntry { id, label: label.to_owned(), enabled: true, children, ..MenuEntry::default() }
    }

    fn event(target: u32, kind: PointerKind) -> PointerEvent {
        PointerEvent { target: Some(target), kind, x: 0.0, y: 0.0, width: 0.0, height: 0.0 }
    }

    #[test]
    fn every_icon_is_in_the_font() {
        for name in ["chevron_left", "chevron_right", "arrow_back", "check"] {
            assert!(shell_m3e::icons::font().contains(name), "{name}");
        }
    }

    #[test]
    fn clicks_go_to_the_item_and_right_clicks_ask_for_its_menu() {
        let mut tray = Tray::new();
        tray.set_items(vec![item("a"), Item { has_menu: false, ..item("b") }]);
        let click = |tray: &mut Tray, index: u32, kind| {
            tray.pointer_event(event(target::ITEM + index, kind))
        };
        assert_eq!(click(&mut tray, 0, PointerKind::Click), [Action::Activate("a".into())]);
        assert_eq!(
            click(&mut tray, 0, PointerKind::SecondaryPress),
            [Action::RequestMenu("a".into())]
        );
        assert_eq!(
            click(&mut tray, 1, PointerKind::SecondaryPress),
            [Action::ContextMenu("b".into())]
        );
        assert_eq!(
            click(&mut tray, 1, PointerKind::MiddlePress),
            [Action::SecondaryActivate("b".into())]
        );
        // An item that has gone takes nothing.
        tray.set_items(vec![item("b")]);
        assert_eq!(click(&mut tray, 0, PointerKind::Click), []);
    }

    #[test]
    fn a_menu_opens_into_its_submenus_and_back() {
        let mut tray = Tray::new();
        tray.set_items(vec![item("a")]);
        let entries = vec![
            entry(1, "Open", vec![]),
            entry(2, "More", vec![entry(3, "Deep", vec![])]),
            MenuEntry { enabled: false, ..entry(4, "Off", vec![]) },
        ];
        let actions = tray.show_menu("a".into(), entries, 300.0, 1920.0);
        assert_eq!(actions, [Action::OpenScrim, Action::OpenMenu]);
        assert!(tray.menu_is_open_for("a"));
        assert_eq!(tray.right, 300.0 - MENU_WIDTH / 2.0);

        // A disabled entry does nothing; one with a menu of its own opens it.
        assert_eq!(tray.menu_event(event(target::ENTRY + 2, PointerKind::Click)), []);
        assert_eq!(tray.menu_event(event(target::ENTRY + 1, PointerKind::Click)), []);
        assert_eq!(tray.menu.as_ref().unwrap().level().1[0].label, "Deep");
        assert_eq!(tray.menu_event(event(target::BACK, PointerKind::Click)), []);
        assert_eq!(tray.menu.as_ref().unwrap().level().1.len(), 3);

        tray.menu_event(event(target::ENTRY + 1, PointerKind::Click));
        assert_eq!(
            tray.menu_event(event(target::ENTRY, PointerKind::Click)),
            [Action::Choose("a".into(), 3)]
        );
        assert!(!tray.menu_is_open());
    }

    #[test]
    fn folding_the_drawer_away_closes_the_menu() {
        let mut tray = Tray::new();
        tray.set_items(vec![item("a")]);
        tray.pointer_event(event(target::CHEVRON, PointerKind::Click));
        assert!(tray.expanded);
        tray.show_menu("a".into(), vec![entry(1, "Open", vec![])], 300.0, 1920.0);
        assert!(tray.menu_is_open());
        tray.pointer_event(event(target::CHEVRON, PointerKind::Click));
        assert!(!tray.expanded && !tray.menu_is_open());
    }

    #[test]
    fn a_menu_goes_with_its_item_and_keeps_to_the_screen() {
        let mut tray = Tray::new();
        tray.set_items(vec![item("a")]);
        tray.show_menu("a".into(), vec![entry(1, "Open", vec![])], 20.0, 1920.0);
        assert_eq!(tray.right, EDGE);
        tray.set_items(Vec::new());
        assert!(!tray.menu_is_open());
        // Nothing to show for an item that isn't there.
        assert_eq!(tray.show_menu("a".into(), vec![entry(1, "Open", vec![])], 20.0, 1920.0), []);
    }
}
