//! Symbols: glyphs of a variable icon font (such as Material Symbols), picked by
//! ligature name and drawn at any point on the font's variation axes.
//!
//! Text goes through cosmic-text, which only drives a variable font's weight. Icon
//! fonts are designed around more axes (Material Symbols has fill, weight, grade and
//! optical size), so symbols are shaped and rasterized here with swash directly.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use swash::{
    FontRef, Setting, Tag,
    scale::{Render, ScaleContext, Source, image::Image},
    shape::ShapeContext,
    text::Script,
    zeno::{Format, Vector},
};

use crate::scene::Color;

/// An icon font: the bytes of an OpenType font whose icons are ligatures of their
/// names, as in Material Symbols. Two `SymbolFont`s are the same font when they share
/// the same bytes.
#[derive(Clone, Copy)]
pub struct SymbolFont(&'static [u8]);

impl SymbolFont {
    pub const fn new(data: &'static [u8]) -> Self {
        Self(data)
    }

    /// Whether the font has a symbol called `name`.
    pub fn contains(self, name: &str) -> bool {
        self.font_ref().and_then(|font| lookup(&mut ShapeContext::new(), font, name)).is_some()
    }

    /// Where symbol `name` actually draws within its `size` em square, with these axis
    /// settings: `[left, top, right, bottom]` in logical pixels from the square's top-left
    /// corner. Icon fonts draw some glyphs well inside the square and others nearly to
    /// its edges, so this is what to space them by to look even. `None` if the font
    /// lacks the name or the glyph is empty. Results are cached.
    pub fn ink_bounds(self, name: &'static str, size: f32, axes: &[Axis]) -> Option<[f32; 4]> {
        static MEASURER: LazyLock<Mutex<Measurer>> = LazyLock::new(Mutex::default);
        let key = (self.id(), name, size.to_bits(), axes.iter().map(axis_key).collect::<Vec<_>>());
        let mut measurer = MEASURER.lock().unwrap();
        if let Some(&bounds) = measurer.cache.get(&key) {
            return bounds;
        }
        let bounds = measurer.measure(self, name, size, axes);
        measurer.cache.insert(key, bounds);
        bounds
    }

    fn id(self) -> usize {
        self.0.as_ptr() as usize
    }

    fn font_ref(self) -> Option<FontRef<'static>> {
        FontRef::from_index(self.0, 0)
    }
}

impl PartialEq for SymbolFont {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl std::fmt::Debug for SymbolFont {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "SymbolFont({} bytes at {:#x})", self.0.len(), self.id())
    }
}

/// A setting of one of a font's variation axes, such as `Axis::new(*b"FILL", 1.0)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Axis {
    pub tag: [u8; 4],
    pub value: f32,
}

impl Axis {
    pub const fn new(tag: [u8; 4], value: f32) -> Self {
        Self { tag, value }
    }
}

/// Up to this many axes can be set on a symbol.
pub const MAX_AXES: usize = 4;

/// A symbol, filling a `size` by `size` square (the font's em square) centered on
/// `center`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Symbol {
    pub font: SymbolFont,
    /// The ligature that names it, such as `"wifi"`.
    pub name: &'static str,
    pub center: [f32; 2],
    /// Logical pixels.
    pub size: f32,
    /// Axis settings; unset slots have a zero tag. Axes left out keep their defaults.
    pub axes: [Axis; MAX_AXES],
    pub color: Color,
}

/// What identifies one rasterized symbol in the glyph atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SymbolKey {
    font: usize,
    glyph: u16,
    /// Physical pixels.
    size: u32,
    /// Axis values in 1/32 steps, which keeps an animated axis to a few dozen images.
    axes: [([u8; 4], i32); MAX_AXES],
}

/// Shapes and rasterizes symbols.
#[derive(Default)]
pub(crate) struct Symbols {
    shaper: ShapeContext,
    scaler: ScaleContext,
    /// Glyph for each (font, name), or `None` if the font has no such ligature.
    glyphs: HashMap<(usize, &'static str), Option<u16>>,
}

/// Where to draw a symbol's image, and how to find or make it.
pub(crate) struct Placed {
    pub key: SymbolKey,
    /// The em square's top-left corner, in whole physical pixels.
    pub origin: [i32; 2],
}

impl Symbols {
    /// Resolves `symbol` at `scale`. `None` if its font lacks the name.
    pub fn place(&mut self, symbol: &Symbol, scale: f32) -> Option<Placed> {
        let glyph = self.glyph(symbol.font, symbol.name)?;
        let size = (symbol.size * scale).round().max(1.0);
        let origin = symbol.center.map(|c| (c * scale - size / 2.0).round() as i32);
        let mut axes = [([0; 4], 0); MAX_AXES];
        for (slot, axis) in axes.iter_mut().zip(&symbol.axes) {
            *slot = (axis.tag, (axis.value * 32.0).round() as i32);
        }
        let key = SymbolKey { font: symbol.font.id(), glyph, size: size as u32, axes };
        Some(Placed { key, origin })
    }

    fn glyph(&mut self, font: SymbolFont, name: &'static str) -> Option<u16> {
        let shaper = &mut self.shaper;
        *self
            .glyphs
            .entry((font.id(), name))
            .or_insert_with(|| lookup(shaper, font.font_ref()?, name))
    }

    /// Rasterizes a symbol as a coverage mask whose placement is relative to the top-left
    /// corner of its em square.
    pub fn rasterize(&mut self, font: SymbolFont, key: SymbolKey) -> Option<Image> {
        let font = font.font_ref()?;
        let settings: Vec<Setting<f32>> = key
            .axes
            .iter()
            .filter(|(tag, _)| *tag != [0; 4])
            .map(|&(tag, value)| Setting {
                tag: Tag::from_be_bytes(tag),
                value: value as f32 / 32.0,
            })
            .collect();
        let mut scaler = self
            .scaler
            .builder(font)
            .size(key.size as f32)
            .hint(false)
            .variations(settings.iter().copied())
            .build();
        // Center the em square vertically on the line the font's ascent and descent span.
        let metrics =
            font.metrics(scaler_coords(&font, &settings).as_slice()).scale(key.size as f32);
        let baseline = (key.size as f32 + metrics.ascent - metrics.descent) / 2.0;
        let mut image = Render::new(&[Source::Outline])
            .format(Format::Alpha)
            .offset(Vector::new(0.0, 0.0))
            .render(&mut scaler, key.glyph)?;
        image.placement.top -= baseline.round() as i32;
        Some(image)
    }
}

type MeasureKey = (usize, &'static str, u32, Vec<([u8; 4], u32)>);

/// Measures symbols' ink for [`SymbolFont::ink_bounds`].
#[derive(Default)]
struct Measurer {
    shaper: ShapeContext,
    scaler: ScaleContext,
    cache: HashMap<MeasureKey, Option<[f32; 4]>>,
}

impl Measurer {
    fn measure(
        &mut self,
        font: SymbolFont,
        name: &'static str,
        size: f32,
        axes: &[Axis],
    ) -> Option<[f32; 4]> {
        let font_ref = font.font_ref()?;
        let glyph = lookup(&mut self.shaper, font_ref, name)?;
        let settings = settings(axes);
        let mut scaler = self
            .scaler
            .builder(font_ref)
            .size(size)
            .hint(false)
            .variations(settings.iter().copied())
            .build();
        let outline = scaler.scale_outline(glyph)?;
        let bounds = outline.bounds();
        if bounds.is_empty() {
            return None;
        }
        // Outlines are y-up from the baseline, which sits as in `Symbols::rasterize`.
        let metrics = font_ref.metrics(scaler_coords(&font_ref, &settings).as_slice()).scale(size);
        let baseline = (size + metrics.ascent - metrics.descent) / 2.0;
        Some([bounds.min.x, baseline - bounds.max.y, bounds.max.x, baseline - bounds.min.y])
    }
}

fn axis_key(axis: &Axis) -> ([u8; 4], u32) {
    (axis.tag, axis.value.to_bits())
}

/// Swash settings for the axes that are set.
fn settings(axes: &[Axis]) -> Vec<Setting<f32>> {
    axes.iter()
        .filter(|axis| axis.tag != [0; 4])
        .map(|axis| Setting { tag: Tag::from_be_bytes(axis.tag), value: axis.value })
        .collect()
}

/// The glyph `name` shapes into as a ligature.
fn lookup(shaper: &mut ShapeContext, font: FontRef, name: &str) -> Option<u16> {
    let mut shaper =
        shaper.builder(font).script(Script::Latin).features(&[("liga", 1), ("rlig", 1)]).build();
    shaper.add_str(name);
    let mut glyphs = Vec::new();
    shaper.shape_with(|cluster| glyphs.extend(cluster.glyphs.iter().map(|g| g.id)));
    // A name that isn't a ligature shapes into one glyph per letter.
    match glyphs[..] {
        [glyph] if glyph != 0 => Some(glyph),
        _ => None,
    }
}

/// Normalized variation coordinates for `settings`, for reading metrics.
fn scaler_coords(font: &FontRef, settings: &[Setting<f32>]) -> Vec<i16> {
    font.variations().normalized_coords(settings.iter().map(|s| (s.tag, s.value))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font with a symbol in it: Material Symbols, which `nix develop` provides.
    fn font() -> Option<SymbolFont> {
        let path = std::env::var("MATERIAL_SYMBOLS_ROUNDED").ok()?;
        Some(SymbolFont::new(std::fs::read(path).ok()?.leak()))
    }

    #[test]
    fn ink_lies_within_the_em_square() {
        let Some(font) = font() else { return };
        let [left, top, right, bottom] = font.ink_bounds("power_settings_new", 24.0, &[]).unwrap();
        assert!(0.0 <= left && left < right && right <= 24.0, "{left}..{right}");
        assert!(0.0 <= top && top < bottom && bottom <= 24.0, "{top}..{bottom}");
        assert_eq!(font.ink_bounds("not_an_icon_name", 24.0, &[]), None);
    }
}
