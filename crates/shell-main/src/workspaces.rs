//! The workspace switcher: each workspace shows its windows' app icons, and a circle
//! travels to the active one.
//!
//! An empty workspace is a tiny faint dot. One with windows shows its app's icon for
//! each window; two or more sit together in a group, a little lighter than the track.
//! Like the track, groups are translucent veils, so the bar's blur shows through.
//! Icons grow in and shrink away as windows open and close.
//!
//! The active workspace collapses: its icons slide together into the middle and fade
//! out, and its group narrows to a single slot, under a `primary` circle. On switching,
//! the circle travels across the row on Material's expressive spatial spring (landing
//! with a touch of overshoot) while the old workspace's group opens back out. The
//! circle tracks the slots as they resize, so it's always over the right one.
//!
//! Each workspace has its own symbol, a Material 3 Expressive shape. On the way the
//! circle morphs, as Material's shapes do: it springs into the symbol of the workspace
//! it's going to, rolls along the track (by whole turns, so it lands upright), and
//! springs back into a circle as it arrives. The mark in its middle morphs into that
//! symbol on the way and keeps it while the workspace is active.

use std::f32::consts::TAU;

use shell_m3e::{
    MaterialShape, ThemeExt,
    motion::{Spring, expressive, standard},
};
use shell_services::{apps::AppIcons, hyprland::Workspaces};
use shell_ui::{AlignItems, Element, Frame, Image, ImageFile, PointerEvent, PointerKind, Shape};

use crate::status::pill_fill;

/// Workspaces shown even if they don't exist yet.
const MIN_SLOTS: i64 = 5;
/// At most this many icons per workspace: its first windows'.
const MAX_ICONS: usize = 4;
/// App icons: the same size as the status pill's icons.
const ICON: f32 = 24.0;
/// Space between icons in a group.
const ICON_GAP: f32 = 5.0;
/// An empty workspace's dot.
const DOT: f32 = 5.0;
/// How strongly a group's background, a veil of `on_secondary_container` (the light tone
/// of the track's color), lightens the track.
const GROUP_ALPHA: f32 = 0.06;
/// How strongly an empty workspace's dot shows: faint, like the pill it's on.
const EMPTY_DOT_ALPHA: f32 = 0.4;
/// The active workspace's circle: a little larger than an icon, so it covers one. It has
/// a mark in the middle, in `on_primary`, in the shape it last morphed through.
const CIRCLE: f32 = 30.0;
/// The mark in the circle: large enough for its shape to read.
const MARK: f32 = 18.0;
/// Every slot is padded this much around its icons, whether or not it shows a group
/// background there, so groups and lone icons line up.
const SLOT_PADDING: f32 = (CIRCLE - ICON) / 2.0;
/// Each workspace's symbol, from 1 up: Material shapes, each a distinct silhouette, most
/// with as many sides or lobes as the workspace's number.
const SHAPES: [MaterialShape; 9] = [
    MaterialShape::Cookie6Sided,
    MaterialShape::Gem,
    MaterialShape::Pentagon,
    MaterialShape::Cookie4Sided,
    MaterialShape::Triangle,
    MaterialShape::Pill,
    MaterialShape::Cookie7Sided,
    MaterialShape::Clover8Leaf,
    MaterialShape::Cookie9Sided,
];
/// Workspaces past the ninth take these in turn.
const MORE_SHAPES: [MaterialShape; 4] = [
    MaterialShape::Cookie12Sided,
    MaterialShape::Arch,
    MaterialShape::PuffyDiamond,
    MaterialShape::Clover4Leaf,
];

/// Workspace `id`'s symbol.
fn shape_of(id: i64) -> MaterialShape {
    match usize::try_from(id - 1) {
        Ok(index) if index < SHAPES.len() => SHAPES[index],
        Ok(index) => MORE_SHAPES[(index - SHAPES.len()) % MORE_SHAPES.len()],
        Err(_) => MaterialShape::Sunny,
    }
}
/// Empty and active workspaces' slots: just wide enough for the circle.
const SLOT: f32 = CIRCLE;
/// Height of the track, matching the status pill.
const HEIGHT: f32 = crate::status::HEIGHT;
/// The track's padding, and the space between slots: group backgrounds and the circle
/// are as far from the track's ends as from its top and bottom, and from each other.
const PADDING: f32 = (HEIGHT - SLOT) / 2.0;
const GAP: f32 = PADDING;

/// What the pointer is over when it's over the indicator.
const TARGET: u32 = 30;

pub struct WorkspaceIndicator {
    slots: Vec<Slot>,
    /// How far the wheel has turned over the indicator since it last switched workspace,
    /// in steps: a touchpad turns it a little at a time.
    scrolled: f32,
    /// Which slot the circle is over, by index; fractions are on the way between two.
    circle: Spring,
    /// 0 → 1 as the circle morphs into `outer` on leaving a workspace, and back as it
    /// arrives at the next.
    morph: Spring,
    /// What the circle morphs into in flight: the symbol of where it's going (or, if
    /// switched again mid-morph, of where it was first going, rather than jump).
    outer: MaterialShape,
    /// The active workspace's symbol, which the mark takes on.
    shape: MaterialShape,
    /// The mark's shape before that, which it morphs out of.
    previous_shape: MaterialShape,
    /// 0 → 1 as the mark morphs from `previous_shape` into `shape`.
    mark: Spring,
    /// How far the circle (and its mark) is turned, clockwise in radians: 0, upright, at
    /// rest.
    angle: f32,
    /// The roll it's making between workspaces, if it's moving.
    roll: Option<Roll>,
    icons: AppIcons,
}

/// The circle rolling from one workspace to another: it turns as it travels, in the
/// direction it's going, by whole turns, so it lands upright.
struct Roll {
    /// Where it set off from, as a slot index.
    from: f32,
    /// Its angle then.
    start: f32,
    /// The angle it lands at, once known: the next frame's layout says how far it goes.
    end: Option<f32>,
}

struct Slot {
    id: i64,
    /// 0 → 1 as it becomes the active workspace: it collapses under the circle.
    active: Spring,
    /// 0 → 1 as windows open on it: the dot brightens.
    occupied: Spring,
    /// Its windows' icons, including any still shrinking away.
    icons: Vec<SlotIcon>,
}

struct SlotIcon {
    /// The window's address.
    window: String,
    file: ImageFile,
    /// 0 → 1 as the window opens on the workspace: the icon grows in.
    presence: Spring,
}

impl Slot {
    fn new(id: i64) -> Self {
        Self {
            id,
            active: Spring::new(standard::SPATIAL, 0.0),
            occupied: Spring::new(expressive::EFFECTS, 0.0),
            icons: Vec::new(),
        }
    }

    /// Shows an icon for each of `windows` (address and icon), in order: new ones grow
    /// in, gone ones shrink away where they were.
    fn set_icons(&mut self, windows: Vec<(String, ImageFile)>) {
        let mut icons: Vec<SlotIcon> = Vec::with_capacity(windows.len());
        for (window, file) in windows {
            let existing = self.icons.iter().position(|icon| icon.window == window);
            let mut icon = match existing {
                Some(index) => self.icons.remove(index),
                None => SlotIcon {
                    window,
                    file,
                    presence: Spring::new(standard::SPATIAL, 0.0).with_precision(0.005),
                },
            };
            icon.presence.set_target(1.0);
            icons.push(icon);
        }
        for (index, mut icon) in self.icons.drain(..).enumerate() {
            icon.presence.set_target(0.0);
            icons.insert(index.min(icons.len()), icon);
        }
        self.icons = icons;
    }
}

impl WorkspaceIndicator {
    /// The circle's angle at `position`, rolling between slots centered at `centers`.
    fn roll(&mut self, centers: &[f32], position: f32) -> f32 {
        let Some(roll) = &mut self.roll else { return self.angle };
        let target = self.circle.target();
        let end = *roll.end.get_or_insert_with(|| {
            // As many turns as rolling that far would make, but at least one.
            let distance = interpolate(centers, target) - interpolate(centers, roll.from);
            let turns = (distance / (TAU * CIRCLE / 2.0)).round();
            let turns = if turns == 0.0 { distance.signum() } else { turns };
            (roll.start / TAU).round() * TAU + turns * TAU
        });
        let span = target - roll.from;
        let progress = if span == 0.0 { 1.0 } else { (position - roll.from) / span };
        self.angle = roll.start + (end - roll.start) * progress;
        if !self.circle.is_moving() {
            self.angle = 0.0;
            self.roll = None;
        }
        self.angle
    }

    /// Shows `state` as is, without animating into it.
    pub fn new(state: &Workspaces) -> Self {
        let mut indicator = Self {
            slots: Vec::new(),
            scrolled: 0.0,
            circle: Spring::new(expressive::SPATIAL, 0.0),
            morph: Spring::new(expressive::SPATIAL_FAST, 0.0),
            outer: shape_of(state.active),
            shape: shape_of(state.active),
            previous_shape: shape_of(state.active),
            mark: Spring::new(expressive::SPATIAL, 1.0),
            angle: 0.0,
            roll: None,
            icons: AppIcons::default(),
        };
        indicator.update(state);
        indicator.circle.snap(indicator.circle.target());
        indicator.morph.set_target(0.0);
        indicator.morph.snap(0.0);
        indicator.previous_shape = indicator.shape;
        indicator.mark.snap(1.0);
        indicator.roll = None;
        for slot in &mut indicator.slots {
            let presences = slot.icons.iter_mut().map(|icon| &mut icon.presence);
            for spring in [&mut slot.active, &mut slot.occupied].into_iter().chain(presences) {
                spring.snap(spring.target());
            }
        }
        indicator
    }

    /// Animates toward `state`.
    pub fn update(&mut self, state: &Workspaces) {
        let last = state.all.iter().map(|ws| ws.id).chain([state.active, MIN_SLOTS]).max();
        let ids = 1..=last.unwrap_or(MIN_SLOTS);

        self.slots.retain(|slot| ids.contains(&slot.id));
        for id in ids {
            if !self.slots.iter().any(|slot| slot.id == id) {
                self.slots.push(Slot::new(id));
            }
        }
        self.slots.sort_by_key(|slot| slot.id);

        if let Some(index) = self.slots.iter().position(|slot| slot.id == state.active) {
            let index = index as f32;
            if index != self.circle.target() {
                let shape = shape_of(state.active);
                // Unless it's still mid-morph, where changing shape would make it jump.
                if self.morph.value().abs() < 0.05 {
                    self.outer = shape;
                }
                self.morph.set_target(1.0);
                self.roll = Some(Roll { from: self.circle.value(), start: self.angle, end: None });
                if shape != self.shape {
                    self.previous_shape = self.shape;
                    self.shape = shape;
                    self.mark.snap(0.0);
                    self.mark.set_target(1.0);
                }
            }
            self.circle.set_target(index);
        }
        for slot in &mut self.slots {
            let workspace = state.all.iter().find(|ws| ws.id == slot.id);
            let clients = workspace.map(|ws| ws.clients.as_slice()).unwrap_or_default();
            let windows = clients
                .iter()
                .filter_map(|client| {
                    let file = ImageFile::new(self.icons.get(&client.class)?);
                    Some((client.address.clone(), file))
                })
                .take(MAX_ICONS)
                .collect();
            slot.set_icons(windows);
            let active = slot.id == state.active;
            let occupied = workspace.is_some_and(|ws| ws.windows > 0);
            slot.active.set_target(if active { 1.0 } else { 0.0 });
            slot.occupied.set_target(if occupied { 1.0 } else { 0.0 });
        }
    }

    /// Takes in what the pointer did on the bar. Scrolling over the indicator steps
    /// through the workspaces, down for the next: returns the one to switch to, if any.
    pub fn pointer_event(&mut self, event: PointerEvent) -> Option<i64> {
        if event.target != Some(TARGET) {
            return None;
        }
        match event.kind {
            PointerKind::Scroll(steps) => self.scrolled += steps,
            PointerKind::Leave => self.scrolled = 0.0,
            _ => {}
        }
        if self.scrolled.abs() < 1.0 {
            return None;
        }
        let forward = self.scrolled > 0.0;
        self.scrolled = 0.0;
        let last = self.slots.len().checked_sub(1)?;
        let current = (self.circle.target().round().max(0.0) as usize).min(last);
        let next = if forward { (current + 1).min(last) } else { current.saturating_sub(1) };
        (next != current).then(|| self.slots[next].id)
    }

    pub fn view(&mut self, frame: &mut Frame) -> Element {
        let theme = frame.theme();
        let mut track = Element::row()
            .align_items(AlignItems::CENTER)
            .height(HEIGHT)
            .target(TARGET)
            .padding_xy(PADDING, 0.0)
            .gap(GAP)
            .background(pill_fill(theme))
            .corner_radius(HEIGHT / 2.0);

        // Each slot's center as laid out this frame, for placing the circle.
        let mut centers = Vec::with_capacity(self.slots.len());
        let mut x = PADDING;
        for slot in &mut self.slots {
            slot.icons.retain(|icon| icon.presence.target() > 0.0 || icon.presence.is_moving());
            let collapse = frame.animate(&mut slot.active).clamp(0.0, 1.0);
            let occupied = frame.animate(&mut slot.occupied);
            let presences: Vec<f32> = slot
                .icons
                .iter_mut()
                .map(|icon| frame.animate(&mut icon.presence).clamp(0.0, 1.0))
                .collect();

            // Laid out open, then narrowed by how collapsed it is.
            let count: f32 = presences.iter().sum();
            let grouped = (count - 1.0).clamp(0.0, 1.0);
            let icons_width = count * ICON + (count - 1.0).max(0.0) * ICON_GAP;
            let open_width = (icons_width + 2.0 * SLOT_PADDING).max(SLOT);
            let width = open_width + (SLOT - open_width) * collapse;
            centers.push(x + width / 2.0);
            x += width + GAP;

            // A light veil over the track, clear unless it's a group.
            let background =
                theme.on_secondary_container.with_alpha(GROUP_ALPHA * grouped * (1.0 - collapse));
            let mut cell = Element::new()
                .size(width, SLOT)
                .shrink(0.0)
                .background(background)
                .corner_radius(SLOT / 2.0);

            let dot = DOT * (1.0 - count.min(1.0));
            if dot > 0.0 {
                cell = cell.child(
                    Element::new()
                        .size(dot, dot)
                        .absolute((width - dot) / 2.0, (SLOT - dot) / 2.0)
                        .background(
                            theme
                                .on_surface_variant
                                .with_alpha(EMPTY_DOT_ALPHA + (1.0 - EMPTY_DOT_ALPHA) * occupied),
                        )
                        .corner_radius(dot / 2.0),
                );
            }

            // Open, icons sit side by side about the middle; collapsed, all in it.
            let mut left = (width - icons_width) / 2.0;
            for (icon, presence) in slot.icons.iter().zip(presences) {
                let size = ICON * presence;
                let open = left;
                let closed = (width - size) / 2.0;
                left += (ICON + ICON_GAP) * presence;
                let opacity = presence * (1.0 - collapse);
                if opacity <= 0.0 || size <= 0.0 {
                    continue;
                }
                cell = cell.child(
                    Element::image(Image::new(icon.file, size).opacity(opacity))
                        .absolute(open + (closed - open) * collapse, (SLOT - size) / 2.0),
                );
            }
            track = track.child(cell);
        }

        // Last, so it's drawn over the slots it passes.
        let position = frame.animate(&mut self.circle);
        // Once it's taken the shape and is nearly there, it turns back into a circle.
        let arriving = (position - self.circle.target()).abs() < 0.5 || !self.circle.is_moving();
        if arriving && self.morph.value() >= 0.9 {
            self.morph.set_target(0.0);
        }
        let morph = frame.animate(&mut self.morph);
        let center = interpolate(&centers, position);
        let rotation = self.roll(&centers, position);
        let shape = Shape::morph(MaterialShape::Circle, self.outer, morph).rotated(rotation);
        // The mark takes on the shape as the circle passes through it, and keeps it.
        let mark = frame.animate(&mut self.mark);
        let mark = Shape::morph(self.previous_shape, self.shape, mark).rotated(rotation);
        track.child(
            Element::new()
                .size(CIRCLE, CIRCLE)
                .absolute(center - CIRCLE / 2.0, (HEIGHT - CIRCLE) / 2.0)
                .background(theme.primary)
                .shape(shape)
                .child(
                    Element::new()
                        .size(MARK, MARK)
                        .absolute((CIRCLE - MARK) / 2.0, (CIRCLE - MARK) / 2.0)
                        .background(theme.on_primary)
                        .shape(mark),
                ),
        )
    }
}

/// The point `index` of the way along `points`, continuing past either end (springs
/// overshoot) at the spacing there.
fn interpolate(points: &[f32], index: f32) -> f32 {
    match points {
        [] => 0.0,
        [only] => *only,
        _ => {
            let last = points.len() - 2;
            let i = (index.floor().max(0.0) as usize).min(last);
            points[i] + (points[i + 1] - points[i]) * (index - i as f32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_between_and_beyond_points() {
        let points = [10.0, 20.0, 40.0];
        assert_eq!(interpolate(&points, 0.0), 10.0);
        assert_eq!(interpolate(&points, 0.5), 15.0);
        assert_eq!(interpolate(&points, 2.0), 40.0);
        assert_eq!(interpolate(&points, 2.1), 42.0);
        assert_eq!(interpolate(&points, -0.1), 9.0);
    }

    #[test]
    fn workspaces_have_their_own_symbols() {
        let first: Vec<_> = (1..=13).map(shape_of).collect();
        for (i, shape) in first.iter().enumerate() {
            assert!(!first[..i].contains(shape), "workspace {} repeats {shape:?}", i + 1);
        }
        assert_eq!(shape_of(3), MaterialShape::Pentagon);
        assert_eq!(shape_of(14), shape_of(10));
    }
}
