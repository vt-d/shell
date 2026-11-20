//! Turns an [`Element`] tree into a taffy tree, lays it out, and paints it into a scene.

use std::collections::HashMap;

use shell_render::cosmic_text::{
    self, Attrs, Buffer, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Metrics, Shaping,
    Weight, Wrap,
};
use taffy::{AvailableSpace, Dimension, NodeId, Position, Size, Style, TaffyTree};

use shell_render::{Color, Morph, Quad, Rect, Scene, TextRun};

use crate::element::Wave;

use crate::element::{Element, FontFamily, Image, Shape, Symbol, TextStyle};

/// Paint properties carried alongside each taffy node.
struct Node {
    background: Option<Color>,
    corner_radii: [f32; 4],
    shape: Option<Shape>,
    wave: Option<Wave>,
    clip: bool,
    target: Option<u32>,
    offset: [f32; 2],
    text: Option<Text>,
    /// With the text color it is drawn in.
    symbol: Option<(Symbol, Color)>,
    /// With the text color, for tinting.
    image: Option<(Image, Color)>,
}

/// An element's style for taffy. Every element is made a containing block, so that
/// [absolute](Element::absolute) children are placed relative to their parent; taffy's
/// default (`Static`) would place them relative to the nearest non-static ancestor.
fn taffy_style(style: &Style) -> Style {
    let mut style = style.clone();
    if style.position == Position::Static {
        style.position = Position::Relative;
    }
    style
}

/// A text leaf: its shaped buffer, re-wrapped whenever layout offers it a new width.
struct Text {
    key: TextKey,
    buffer: Buffer,
    color: Color,
    /// Height of the laid-out lines at the final width.
    height: f32,
    /// The width (as bits; `None` is unbounded) the buffer is wrapped to now.
    wrapped_to: Option<Option<u32>>,
    /// The extent of the lines at each width tried so far: laying a view out again for
    /// each frame of an animation asks for the same few every time.
    extents: Vec<(Option<u32>, (f32, f32))>,
}

/// What a text leaf's shaping depends on: its content and all of its style but color.
#[derive(Clone, PartialEq, Eq, Hash)]
struct TextKey {
    content: String,
    size: u32,
    line_height: u32,
    family: FontFamily,
    weight: u16,
    italic: bool,
    wrap: bool,
    ellipsis: bool,
}

/// Shaped text from a tree being replaced, for the tree replacing it to take up.
type Spare = HashMap<TextKey, Vec<Text>>;

pub(crate) struct Layout {
    tree: TaffyTree<Node>,
    root: NodeId,
    /// The root's style as authored, before the window's size is imposed on it.
    root_style: Style,
    /// Constraints of the last computation, to skip recomputing when unchanged.
    computed_for: Option<(Option<f32>, Option<f32>)>,
}

impl Layout {
    /// Builds the tree for `root`.
    pub fn new(root: &Element) -> Self {
        Self::build(root, &mut Spare::new())
    }

    /// Builds the tree for `root` in place of this one, keeping the shaped text the two
    /// have in common: shaping is most of what a view rebuilt for each frame of an
    /// animation would cost.
    pub fn rebuild(&mut self, root: &Element) {
        let mut spare = Spare::new();
        let mut stack = vec![self.root];
        while let Some(node) = stack.pop() {
            stack.extend(self.tree.children(node).expect("node exists"));
            let text = self.tree.get_node_context_mut(node).and_then(|n| n.text.take());
            if let Some(text) = text {
                spare.entry(text.key.clone()).or_default().push(text);
            }
        }
        *self = Self::build(root, &mut spare);
    }

    fn build(root: &Element, spare: &mut Spare) -> Self {
        let mut tree = TaffyTree::new();
        let root_id = insert(&mut tree, root, &ResolvedTextStyle::default(), 1.0, spare);
        Self { tree, root: root_id, root_style: taffy_style(&root.style), computed_for: None }
    }

    /// Lays out the tree. A `Some` axis forces the root to that many logical pixels;
    /// `None` sizes it to its content. Returns the root's resulting size.
    pub fn compute(
        &mut self,
        fonts: &mut FontSystem,
        width: Option<f32>,
        height: Option<f32>,
    ) -> (f32, f32) {
        if self.computed_for != Some((width, height)) {
            let mut style = self.root_style.clone();
            if let Some(w) = width {
                style.size.width = Dimension::length(w);
            }
            if let Some(h) = height {
                style.size.height = Dimension::length(h);
            }
            let available = Size {
                width: width.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
                height: height.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
            };
            self.tree.set_style(self.root, style).expect("root node exists");
            self.tree
                .compute_layout_with_measure(self.root, available, |inputs, _, node, style| {
                    taffy::compute_leaf_layout(
                        inputs,
                        style,
                        |_, _| 0.0,
                        |known, available| match node.and_then(|n| n.text.as_mut()) {
                            Some(text) => text.measure(fonts, known, available),
                            None => Size::ZERO,
                        },
                    )
                })
                .expect("taffy layout failed");
            self.finish_text(fonts);
            self.computed_for = Some((width, height));
        }
        let size = self.tree.layout(self.root).expect("root node exists").size;
        (size.width, size.height)
    }

    /// Measuring leaves each buffer wrapped to whatever width was tried last; wrap them
    /// to the width they were finally given.
    fn finish_text(&mut self, fonts: &mut FontSystem) {
        let mut stack = vec![self.root];
        while let Some(node) = stack.pop() {
            stack.extend(self.tree.children(node).expect("node exists"));
            let width = self.tree.layout(node).expect("node exists").content_box_width();
            if let Some(text) = self.tree.get_node_context_mut(node).and_then(|n| n.text.as_mut()) {
                text.wrap_to(fonts, Some(width));
                text.height = text.extent(fonts, Some(width)).1;
            }
        }
    }

    /// Appends the most recent layout to `scene`, back to front, and returns where its
    /// pointer [targets](Element::target) are, in the same order.
    pub fn paint<'a>(&'a self, scene: &mut Scene<'a>) -> Vec<Hit> {
        let mut hits = Vec::new();
        self.paint_node(self.root, 0.0, 0.0, None, scene, &mut hits);
        hits
    }

    fn paint_node<'a>(
        &'a self,
        node: NodeId,
        parent_x: f32,
        parent_y: f32,
        clip: Option<Rect>,
        scene: &mut Scene<'a>,
        hits: &mut Vec<Hit>,
    ) {
        let layout = self.tree.layout(node).expect("node exists");
        // An offset moves the node as though its parent were elsewhere.
        let [dx, dy] = self.tree.get_node_context(node).map_or([0.0; 2], |n| n.offset);
        let (parent_x, parent_y) = (parent_x + dx, parent_y + dy);
        let x = parent_x + layout.location.x;
        let y = parent_y + layout.location.y;
        let (width, height) = (layout.size.width, layout.size.height);
        let rect = Rect { x, y, width, height };

        let mut clip = clip;
        let mut clipped = false;
        if let Some(paint) = self.tree.get_node_context(node) {
            match (paint.background, paint.shape, paint.wave) {
                (Some(color), _, Some(wave)) => scene.wave(shell_render::Wave {
                    rect,
                    color,
                    amplitude: wave.amplitude,
                    wavelength: wave.wavelength,
                    phase: wave.phase,
                    thickness: wave.thickness,
                }),
                (Some(color), None, None) => {
                    scene.quad(Quad { rect, color, corner_radii: paint.corner_radii })
                }
                (Some(color), Some(shape), None) => scene.morph(Morph {
                    center: [x + width / 2.0, y + height / 2.0],
                    size: width.min(height) * shape.scale,
                    from: shape.from,
                    to: shape.to,
                    progress: shape.progress,
                    rotation: shape.rotation,
                    color,
                }),
                (None, ..) => {}
            }
            if let Some(id) = paint.target {
                // Only where it shows.
                let rect = clip.map_or(rect, |clip| intersection(rect, clip));
                if rect.width > 0.0 && rect.height > 0.0 {
                    hits.push(Hit { id, rect: Rect { x, y, width, height }, shown: rect });
                }
            }
            if paint.clip {
                scene.push_clip(rect, paint.corner_radii);
                clip = Some(clip.map_or(rect, |outer| intersection(rect, outer)));
                clipped = true;
            }
            if let Some(text) = &paint.text {
                let slack = (layout.content_box_height() - text.height).max(0.0);
                scene.text(TextRun {
                    buffer: &text.buffer,
                    // content_box_{x,y} are relative to the parent, like `location`.
                    origin: [
                        parent_x + layout.content_box_x(),
                        (parent_y + layout.content_box_y() + slack / 2.0).round(),
                    ],
                    color: text.color,
                });
            }
            if let Some((symbol, color)) = paint.symbol {
                scene.symbol(shell_render::Symbol {
                    font: symbol.font,
                    name: symbol.name,
                    center: [x + width / 2.0, y + height / 2.0],
                    size: symbol.size,
                    axes: symbol.axes,
                    color,
                });
            }
            if let Some((image, color)) = paint.image {
                scene.image(shell_render::Image {
                    file: image.file,
                    center: [x + width / 2.0, y + height / 2.0],
                    size: image.size,
                    opacity: image.opacity,
                    tint: image.tinted.then_some(color),
                    cover: image.cover,
                });
            }
        }
        for child in self.tree.children(node).expect("node exists") {
            self.paint_node(child, x, y, clip, scene, hits);
        }
        if clipped {
            scene.pop_clip();
        }
    }
}

/// Where a pointer [target](Element::target) was painted.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub id: u32,
    /// Its box, in the window's logical pixels.
    pub rect: Rect,
    /// The part of its box that shows (isn't clipped away), which is what takes input.
    pub shown: Rect,
}

impl Hit {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let r = self.shown;
        x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
    }
}

/// The rectangle two share; empty (zero-sized) if they don't overlap.
fn intersection(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    Rect { x, y, width: (right - x).max(0.0), height: (bottom - y).max(0.0) }
}

impl Text {
    /// The text for `content` in `style`: one of the `spare` if any is the same, else
    /// shaped anew.
    fn new(content: &str, style: &ResolvedTextStyle, color: Color, spare: &mut Spare) -> Self {
        let key = TextKey {
            content: content.to_owned(),
            size: style.size.to_bits(),
            line_height: style.line_height.to_bits(),
            family: style.family.clone(),
            weight: style.weight,
            italic: style.italic,
            wrap: style.wrap,
            ellipsis: style.ellipsis,
        };
        if let Some(mut text) = spare.get_mut(&key).and_then(Vec::pop) {
            text.color = color;
            return text;
        }
        let mut buffer = Buffer::new_empty(Metrics::relative(style.size, style.line_height));
        // Word wrapping keeps min-content at the longest word, as in CSS.
        buffer.set_wrap(if style.wrap && !style.ellipsis { Wrap::Word } else { Wrap::None });
        if style.ellipsis {
            buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)));
        }
        let family = match &style.family {
            FontFamily::SansSerif => Family::SansSerif,
            FontFamily::Serif => Family::Serif,
            FontFamily::Monospace => Family::Monospace,
            FontFamily::Name(name) => Family::Name(name),
        };
        let attrs =
            Attrs::new().family(family).weight(Weight(style.weight)).style(if style.italic {
                cosmic_text::Style::Italic
            } else {
                cosmic_text::Style::Normal
            });
        buffer.set_text(content, &attrs, Shaping::Advanced, None);
        Self { key, buffer, color, height: 0.0, wrapped_to: None, extents: Vec::new() }
    }

    fn measure(
        &mut self,
        fonts: &mut FontSystem,
        known: Size<Option<f32>>,
        available: Size<AvailableSpace>,
    ) -> Size<f32> {
        let width = known.width.or(match available.width {
            AvailableSpace::Definite(w) => Some(w),
            AvailableSpace::MinContent => Some(0.0),
            AvailableSpace::MaxContent => None,
        });
        let (w, h) = self.extent(fonts, width);
        // Rounded up so that, laid out again at exactly this width, nothing re-wraps.
        Size { width: known.width.unwrap_or(w.ceil()), height: known.height.unwrap_or(h.ceil()) }
    }

    /// How many widths to remember the extent at: layout tries only a few.
    const EXTENTS: usize = 8;

    /// The extent of the lines when wrapped to `width` (unbounded if `None`). Leaves the
    /// buffer wrapped to whatever width was last worked out; see [`Text::wrap_to`].
    fn extent(&mut self, fonts: &mut FontSystem, width: Option<f32>) -> (f32, f32) {
        let key = width.map(f32::to_bits);
        if let Some(&(_, extent)) = self.extents.iter().find(|(at, _)| *at == key) {
            return extent;
        }
        self.wrap_to(fonts, width);
        let extent = self.buffer.layout_runs().fold((0.0, 0.0), |(w, h), run| {
            (f32::max(w, run.line_w), f32::max(h, run.line_top + run.line_height))
        });
        if self.extents.len() == Self::EXTENTS {
            self.extents.clear();
        }
        self.extents.push((key, extent));
        extent
    }

    /// Wraps the buffer to `width`, if it isn't already. Shaping is cached by the buffer,
    /// so only line breaking is redone.
    fn wrap_to(&mut self, fonts: &mut FontSystem, width: Option<f32>) {
        let key = width.map(f32::to_bits);
        if self.wrapped_to != Some(key) {
            self.buffer.set_size(width, None);
            self.buffer.shape_until_scroll(fonts, false);
            self.wrapped_to = Some(key);
        }
    }
}

/// A [`TextStyle`] with every property decided.
struct ResolvedTextStyle {
    size: f32,
    line_height: f32,
    color: Color,
    family: FontFamily,
    weight: u16,
    italic: bool,
    wrap: bool,
    ellipsis: bool,
}

impl Default for ResolvedTextStyle {
    fn default() -> Self {
        Self {
            size: 14.0,
            line_height: 1.25,
            color: Color::WHITE,
            family: FontFamily::SansSerif,
            weight: 400,
            italic: false,
            wrap: true,
            ellipsis: false,
        }
    }
}

impl ResolvedTextStyle {
    fn inherit(&self, own: &TextStyle) -> Self {
        Self {
            size: own.size.unwrap_or(self.size),
            line_height: own.line_height.unwrap_or(self.line_height),
            color: own.color.unwrap_or(self.color),
            family: own.family.clone().unwrap_or_else(|| self.family.clone()),
            weight: own.weight.unwrap_or(self.weight),
            italic: own.italic.unwrap_or(self.italic),
            wrap: own.wrap.unwrap_or(self.wrap),
            ellipsis: own.ellipsis.unwrap_or(self.ellipsis),
        }
    }
}

fn insert(
    tree: &mut TaffyTree<Node>,
    element: &Element,
    inherited: &ResolvedTextStyle,
    parent_opacity: f32,
    spare: &mut Spare,
) -> NodeId {
    let opacity = parent_opacity * element.opacity.unwrap_or(1.0);
    let fade = |color: Color| color.with_alpha(color.a * opacity);
    let text_style = inherited.inherit(&element.text_style);
    let ink = fade(text_style.color);
    let node = Node {
        background: element.background.map(fade),
        corner_radii: element.corner_radii,
        shape: element.shape,
        wave: element.wave,
        clip: element.clip,
        target: element.target,
        offset: element.offset,
        text: element.text.as_deref().map(|content| Text::new(content, &text_style, ink, spare)),
        symbol: element.symbol.map(|symbol| (symbol, ink)),
        image: element
            .image
            .map(|image| (image.opacity(image.opacity * opacity), text_style.color)),
    };
    let id = if node.text.is_some() || node.symbol.is_some() || node.image.is_some() {
        tree.new_leaf(taffy_style(&element.style))
    } else {
        let children: Vec<NodeId> =
            element.children.iter().map(|c| insert(tree, c, &text_style, opacity, spare)).collect();
        tree.new_with_children(taffy_style(&element.style), &children)
    }
    .expect("taffy insert");
    tree.set_node_context(id, Some(node)).expect("node exists");
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use shell_render::ImageFile;

    #[test]
    fn fading_a_page_preserves_its_glass_and_updates_cached_text_and_images() {
        let page = |opacity| {
            Element::column()
                .opacity(opacity)
                .background(Color::BLACK.with_alpha(0.5))
                .text_color(Color::WHITE.with_alpha(0.8))
                .child(Element::text("Bluetooth"))
                .child(Element::image(Image::new(ImageFile::new("avatar.png"), 48.0).opacity(0.6)))
        };
        let mut layout = Layout::new(&page(1.0));
        layout.rebuild(&page(0.5));
        let root = layout.tree.get_node_context(layout.root).unwrap();
        assert_eq!(root.background.unwrap().a, 0.25);
        let children = layout.tree.children(layout.root).unwrap();
        let label = layout.tree.get_node_context(children[0]).unwrap();
        assert_eq!(label.text.as_ref().unwrap().color.a, 0.4);
        let avatar = layout.tree.get_node_context(children[1]).unwrap();
        assert_eq!(avatar.image.unwrap().0.opacity, 0.3);
    }
}
