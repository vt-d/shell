//! The element tree: boxes laid out with flexbox (via taffy) and painted by the renderer.

use taffy::{
    AlignItems, Dimension, FlexDirection, JustifyContent, LengthPercentage, LengthPercentageAuto,
    Position, Style,
};

use shell_render::{Axis, Color, ImageFile, MAX_AXES, Outline, SymbolFont};

/// A glyph of a variable icon font, picked by name: see [`Element::symbol`]. Design
/// systems wrap this (`shell-m3e` has Material Symbols as `Icon`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Symbol {
    pub font: SymbolFont,
    /// Its ligature name, such as `"wifi"`.
    pub name: &'static str,
    /// The em square's side, in logical pixels.
    pub size: f32,
    /// Variation axis settings; unused slots have a zero tag.
    pub axes: [Axis; MAX_AXES],
}

/// An image file (PNG or SVG), such as an app icon: see [`Element::image`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Image {
    pub file: ImageFile,
    /// The square it's fit into, in logical pixels.
    pub size: f32,
    /// 0 is invisible, 1 opaque.
    pub opacity: f32,
    /// Drawn in the text color, keeping only the image's shape: for single-color icons.
    pub tinted: bool,
    /// Fills the square, cropping what doesn't fit, rather than fitting inside it.
    pub cover: bool,
}

impl Image {
    pub fn new(file: ImageFile, size: f32) -> Self {
        Self { file, size, opacity: 1.0, tinted: false, cover: false }
    }

    pub fn opacity(self, opacity: f32) -> Self {
        Self { opacity, ..self }
    }

    /// Fills the square with the image, cropping what of it doesn't fit: for photos and
    /// album art, which should fill their frame whatever their proportions.
    pub fn cover(self) -> Self {
        Self { cover: true, ..self }
    }

    /// Draws it in the text color, as icons usually are.
    pub fn tinted(self) -> Self {
        Self { tinted: true, ..self }
    }
}

/// Which typeface text is set in. Generic families resolve through fontconfig; any font
/// lacking a glyph falls back to one that has it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontFamily {
    #[default]
    SansSerif,
    Serif,
    Monospace,
    /// A family name such as `"Inter"` or `"JetBrains Mono"`.
    Name(String),
}

/// An outline for an element's background other than a (rounded) rectangle: an
/// [`Outline`], or one partway through morphing into another, drawn centered in the
/// element and as large as fits. Every number can be animated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub from: Outline,
    pub to: Outline,
    /// 0 is `from`, 1 is `to`. Values a little outside that range (from a spring's
    /// overshoot) exaggerate the morph.
    pub progress: f32,
    /// Clockwise, in radians, from the outline's own orientation.
    pub rotation: f32,
    /// Size relative to the largest that fits. Animating this rather than the element's
    /// size grows the shape smoothly without reflowing its neighbors.
    pub scale: f32,
}

impl Shape {
    pub fn new(outline: impl Into<Outline>) -> Self {
        let outline = outline.into();
        Self::morph(outline, outline, 0.0)
    }

    pub fn morph(from: impl Into<Outline>, to: impl Into<Outline>, progress: f32) -> Self {
        Self { from: from.into(), to: to.into(), progress, rotation: 0.0, scale: 1.0 }
    }

    pub const fn rotated(mut self, radians: f32) -> Self {
        self.rotation = radians;
        self
    }

    pub const fn scaled(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }
}

impl From<Outline> for Shape {
    fn from(outline: Outline) -> Self {
        Self::new(outline)
    }
}

/// A sine wave stroked along an element's length, about its middle, in its background
/// color, with round ends: see [`Element::wave`]. Every number can be animated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wave {
    /// How far it swings either side of the middle; 0 is a straight line.
    pub amplitude: f32,
    /// The length of one cycle.
    pub wavelength: f32,
    /// Where in its cycle it starts, in radians; animate it to make the wave run.
    pub phase: f32,
    /// The stroke's width.
    pub thickness: f32,
}

/// Text properties set on an element. Unset properties are inherited from the nearest
/// ancestor that sets them, so a font can be chosen once for a whole window.
#[derive(Clone, Debug, Default)]
pub(crate) struct TextStyle {
    pub size: Option<f32>,
    pub line_height: Option<f32>,
    pub color: Option<Color>,
    pub family: Option<FontFamily>,
    pub weight: Option<u16>,
    pub italic: Option<bool>,
    pub wrap: Option<bool>,
    pub ellipsis: Option<bool>,
}

/// A box in a window's UI. Elements are built declaratively and handed to a window
/// as its root; sizes and spacing are in logical pixels.
///
/// ```ignore
/// Element::row()
///     .padding(4.0)
///     .gap(8.0)
///     .font_size(13.0)
///     .text_color(Color::hex(0xcdd6f4))
///     .child(Element::new().width(24.0).background(Color::hex(0x89b4fa)).corner_radius(6.0))
///     .child(Element::text("workspace 1"))
///     .child(Element::spacer())
///     .child(Element::text("12:00").bold())
/// ```
#[derive(Clone, Debug, Default)]
pub struct Element {
    pub(crate) style: Style,
    pub(crate) background: Option<Color>,
    pub(crate) corner_radii: [f32; 4],
    pub(crate) shape: Option<Shape>,
    pub(crate) wave: Option<Wave>,
    pub(crate) clip: bool,
    /// What pointer events over it are reported as being on; see [`Element::target`].
    pub(crate) target: Option<u32>,
    /// Paint-time translation, in logical pixels; see [`Element::offset`].
    pub(crate) offset: [f32; 2],
    pub(crate) text: Option<String>,
    pub(crate) symbol: Option<Symbol>,
    pub(crate) image: Option<Image>,
    pub(crate) text_style: TextStyle,
    pub(crate) children: Vec<Element>,
}

impl Element {
    /// A run of text, sized to fit it. Lines wrap at word boundaries when the element is
    /// narrower than the text (see [`Element::wrap`]); `\n` starts a new line. The text
    /// is centered vertically if the element is taller than it.
    ///
    /// Text elements are leaves: children added to one are ignored.
    pub fn text(content: impl Into<String>) -> Self {
        Self { text: Some(content.into()), ..Self::default() }
    }

    /// An icon from a variable icon font, `symbol.size` square unless sized otherwise
    /// (it stays centered), in the text color. Like text, symbols are leaves.
    pub fn symbol(symbol: impl Into<Symbol>) -> Self {
        let symbol = symbol.into();
        Self { symbol: Some(symbol), ..Self::default() }.size(symbol.size, symbol.size).shrink(0.0)
    }

    /// An image, fit into an `image.size` square (keeping its aspect ratio) and the
    /// element that size unless sized otherwise (it stays centered). Images are leaves.
    pub fn image(image: Image) -> Self {
        Self { image: Some(image), ..Self::default() }.size(image.size, image.size).shrink(0.0)
    }

    /// A flex container laying out children in a row.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn row() -> Self {
        Self::new().direction(FlexDirection::Row)
    }

    pub fn column() -> Self {
        Self::new().direction(FlexDirection::Column)
    }

    /// An empty element that absorbs free space, pushing its siblings apart.
    pub fn spacer() -> Self {
        Self::new().grow(1.0)
    }

    // --- Sizing ---------------------------------------------------------------

    pub fn width(mut self, px: f32) -> Self {
        self.style.size.width = Dimension::length(px);
        self
    }

    pub fn height(mut self, px: f32) -> Self {
        self.style.size.height = Dimension::length(px);
        self
    }

    pub fn size(self, width: f32, height: f32) -> Self {
        self.width(width).height(height)
    }

    /// Caps the width: the element is narrower if its content is, never wider.
    pub fn max_width(mut self, px: f32) -> Self {
        self.style.max_size.width = LengthPercentageAuto::length(px);
        self
    }

    pub fn fill_width(mut self) -> Self {
        self.style.size.width = Dimension::percent(1.0);
        self
    }

    pub fn fill_height(mut self) -> Self {
        self.style.size.height = Dimension::percent(1.0);
        self
    }

    pub fn fill(self) -> Self {
        self.fill_width().fill_height()
    }

    pub fn grow(mut self, factor: f32) -> Self {
        self.style.flex_grow = factor;
        self
    }

    pub fn shrink(mut self, factor: f32) -> Self {
        self.style.flex_shrink = factor;
        self
    }

    // --- Spacing --------------------------------------------------------------

    pub fn padding(self, px: f32) -> Self {
        self.padding_xy(px, px)
    }

    pub fn padding_xy(mut self, horizontal: f32, vertical: f32) -> Self {
        let (h, v) = (LengthPercentage::length(horizontal), LengthPercentage::length(vertical));
        self.style.padding = taffy::Rect { left: h, right: h, top: v, bottom: v };
        self
    }

    /// Padding on each side, in CSS order.
    pub fn padding_sides(mut self, top: f32, right: f32, bottom: f32, left: f32) -> Self {
        let p = LengthPercentage::length;
        self.style.padding =
            taffy::Rect { left: p(left), right: p(right), top: p(top), bottom: p(bottom) };
        self
    }

    pub fn margin(mut self, px: f32) -> Self {
        let m = LengthPercentageAuto::length(px);
        self.style.margin = taffy::Rect { left: m, right: m, top: m, bottom: m };
        self
    }

    /// Space between children.
    pub fn gap(mut self, px: f32) -> Self {
        let g = LengthPercentage::length(px);
        self.style.gap = taffy::Size { width: g, height: g };
        self
    }

    // --- Flex -----------------------------------------------------------------

    pub fn direction(mut self, direction: FlexDirection) -> Self {
        self.style.flex_direction = direction;
        self
    }

    /// Cross-axis alignment of children.
    pub fn align_items(mut self, align: AlignItems) -> Self {
        self.style.align_items = align;
        self
    }

    /// Main-axis distribution of children.
    pub fn justify_content(mut self, justify: JustifyContent) -> Self {
        self.style.justify_content = justify;
        self
    }

    /// Centers children on both axes.
    pub fn center(self) -> Self {
        self.align_items(AlignItems::CENTER).justify_content(JustifyContent::CENTER)
    }

    /// Takes the element out of the flow, placing its top-left corner `x`, `y` from its
    /// parent's top-left corner (inside any border, outside padding). It takes up no
    /// space and is painted in its place among its siblings, so an absolute element
    /// added first sits behind the rest.
    pub fn absolute(mut self, x: f32, y: f32) -> Self {
        self.style.position = Position::Absolute;
        self.style.inset = taffy::Rect {
            left: LengthPercentageAuto::length(x),
            top: LengthPercentageAuto::length(y),
            right: LengthPercentageAuto::auto(),
            bottom: LengthPercentageAuto::auto(),
        };
        self
    }

    /// Escape hatch for any taffy style property without a dedicated builder.
    pub fn style(mut self, f: impl FnOnce(&mut Style)) -> Self {
        f(&mut self.style);
        self
    }

    // --- Paint ----------------------------------------------------------------

    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Rounds every corner by `px`. Radii are limited to half the shorter side, so a
    /// large one makes a pill or circle.
    pub fn corner_radius(self, px: f32) -> Self {
        self.corner_radii(px, px, px, px)
    }

    /// Rounds each corner separately, clockwise from the top left.
    pub fn corner_radii(
        mut self,
        top_left: f32,
        top_right: f32,
        bottom_right: f32,
        bottom_left: f32,
    ) -> Self {
        self.corner_radii = [top_left, top_right, bottom_right, bottom_left];
        self
    }

    /// Moves the element and its children by `dx`, `dy` when painted, without affecting
    /// layout. Layout snaps to whole pixels; animating this instead moves things
    /// smoothly, and without reflowing their neighbors.
    pub fn offset(mut self, dx: f32, dy: f32) -> Self {
        self.offset = [dx, dy];
        self
    }

    /// Paints the background in `shape` instead of a rectangle.
    pub fn shape(mut self, shape: impl Into<Shape>) -> Self {
        self.shape = Some(shape.into());
        self
    }

    /// Paints the background as a wave stroked along the element instead of a rectangle.
    pub fn wave(mut self, wave: Wave) -> Self {
        self.wave = Some(wave);
        self
    }

    /// Cuts off what the element's descendants (and its own text or image) draw outside
    /// its box, rounded by its corner radii.
    pub fn clip(mut self) -> Self {
        self.clip = true;
        self
    }

    // --- Input ----------------------------------------------------------------

    /// Makes the element take pointer input: events over its box (as laid out and
    /// offset) are reported with `id` as their target, to the handler given to
    /// [`Ui::on_pointer`](crate::Ui::on_pointer) for its window. Where targets overlap,
    /// the one painted last (the topmost) gets the event.
    pub fn target(mut self, id: u32) -> Self {
        self.target = Some(id);
        self
    }

    // --- Text (inherited by descendants) --------------------------------------

    /// Font size in logical pixels. Defaults to 14.
    pub fn font_size(mut self, px: f32) -> Self {
        self.text_style.size = Some(px);
        self
    }

    /// Line height as a multiple of the font size. Defaults to 1.25.
    pub fn line_height(mut self, factor: f32) -> Self {
        self.text_style.line_height = Some(factor);
        self
    }

    /// Defaults to white. A design system sets it on a window's root (see `shell-m3e`).
    pub fn text_color(mut self, color: Color) -> Self {
        self.text_style.color = Some(color);
        self
    }

    pub fn font_family(mut self, family: FontFamily) -> Self {
        self.text_style.family = Some(family);
        self
    }

    /// CSS-style weight: 400 is regular, 700 bold. Variable fonts honor any value.
    pub fn font_weight(mut self, weight: u16) -> Self {
        self.text_style.weight = Some(weight);
        self
    }

    pub fn bold(self) -> Self {
        self.font_weight(700)
    }

    pub fn italic(mut self) -> Self {
        self.text_style.italic = Some(true);
        self
    }

    /// Whether lines wrap to the element's width. On by default; with it off, text keeps
    /// to one line per `\n` and its width is never squeezed below the text's.
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.text_style.wrap = Some(wrap);
        self
    }

    /// Whether text too long for the element ends in an ellipsis instead of overflowing.
    /// Off by default. Implies a single line; the element can then be squeezed narrower
    /// than its text, so cap its width or let it shrink.
    pub fn ellipsis(mut self, ellipsis: bool) -> Self {
        self.text_style.ellipsis = Some(ellipsis);
        self
    }

    // --- Children -------------------------------------------------------------

    pub fn child(mut self, child: Element) -> Self {
        self.children.push(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = Element>) -> Self {
        self.children.extend(children);
        self
    }
}
