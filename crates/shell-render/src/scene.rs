//! Backend-agnostic description of a frame: what to draw, in logical pixels.

use crate::{image::Image, outline::Outline, symbol::Symbol};

/// An sRGB color with straight (non-premultiplied) alpha, each channel in `0.0..=1.0`.
/// The default is transparent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::rgba(r, g, b, 1.0)
    }

    /// From a `0xRRGGBB` literal.
    pub const fn hex(rgb: u32) -> Self {
        Self::rgb(
            ((rgb >> 16) & 0xff) as f32 / 255.0,
            ((rgb >> 8) & 0xff) as f32 / 255.0,
            (rgb & 0xff) as f32 / 255.0,
        )
    }

    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// Blends toward `other` by `t` (0 is `self`, 1 is `other`), channel by channel in
    /// sRGB, which keeps the midpoints of light/dark transitions perceptually even.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Self::rgba(
            mix(self.r, other.r),
            mix(self.g, other.g),
            mix(self.b, other.b),
            mix(self.a, other.a),
        )
    }

    /// Linear-light, premultiplied RGBA: what an sRGB render target and premultiplied
    /// blending expect.
    pub(crate) fn to_linear_premultiplied(self) -> [f32; 4] {
        let a = self.a;
        [srgb_to_linear(self.r) * a, srgb_to_linear(self.g) * a, srgb_to_linear(self.b) * a, a]
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A filled, optionally rounded rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub rect: Rect,
    pub color: Color,
    /// Top-left, top-right, bottom-right, bottom-left. Each is limited to half the
    /// shorter side, so a large radius rounds that end fully.
    pub corner_radii: [f32; 4],
}

/// A filled [`Outline`], or one partway through morphing into another, filling a `size`
/// by `size` square centered on `center`. Morphing blends the two outlines' radii angle
/// by angle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Morph {
    pub center: [f32; 2],
    pub size: f32,
    pub from: Outline,
    pub to: Outline,
    /// 0 is `from`, 1 is `to`.
    pub progress: f32,
    /// Clockwise, in radians.
    pub rotation: f32,
    pub color: Color,
}

/// A sine wave stroked along `rect`'s length, about its vertical middle, with round ends:
/// Material's wavy progress track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wave {
    pub rect: Rect,
    pub color: Color,
    /// How far the wave swings either side of the middle; 0 is a straight line.
    pub amplitude: f32,
    /// The length of one cycle.
    pub wavelength: f32,
    /// Where in its cycle the wave starts, in radians; animate it to make the wave run.
    pub phase: f32,
    /// The stroke's width.
    pub thickness: f32,
}

/// A shaped and laid-out block of text, drawn with its top-left corner at `origin`.
#[derive(Clone, Copy, Debug)]
pub struct TextRun<'a> {
    pub buffer: &'a cosmic_text::Buffer,
    pub origin: [f32; 2],
    /// Used for glyphs whose attributes don't carry their own color.
    pub color: Color,
}

#[derive(Clone, Copy, Debug)]
pub enum Primitive<'a> {
    Quad(Quad),
    Morph(Morph),
    Text(TextRun<'a>),
    Symbol(Symbol),
    Image(Image),
    Wave(Wave),
    /// Clips what follows, until the matching [`Primitive::PopClip`], to a rectangle with
    /// these corner radii (clockwise from top-left). Clips nest: a clip inside another
    /// is cut to the rectangle both share, with its own corners.
    PushClip(Rect, [f32; 4]),
    PopClip,
}

/// Everything needed to draw one frame of a surface. Coordinates are logical pixels;
/// the renderer applies the surface's scale factor. Text borrows the buffers it was
/// shaped into, so a scene lives only as long as the layout that painted it.
#[derive(Clone, Debug, Default)]
pub struct Scene<'a> {
    pub clear: Color,
    /// Painted in order, back to front.
    pub primitives: Vec<Primitive<'a>>,
}

impl<'a> Scene<'a> {
    pub fn new(clear: Color) -> Self {
        Self { clear, primitives: Vec::new() }
    }

    pub fn quad(&mut self, quad: Quad) {
        self.primitives.push(Primitive::Quad(quad));
    }

    pub fn morph(&mut self, morph: Morph) {
        self.primitives.push(Primitive::Morph(morph));
    }

    pub fn text(&mut self, text: TextRun<'a>) {
        self.primitives.push(Primitive::Text(text));
    }

    pub fn image(&mut self, image: Image) {
        self.primitives.push(Primitive::Image(image));
    }

    pub fn symbol(&mut self, symbol: Symbol) {
        self.primitives.push(Primitive::Symbol(symbol));
    }

    pub fn wave(&mut self, wave: Wave) {
        self.primitives.push(Primitive::Wave(wave));
    }

    /// Clips what is added until [`Scene::pop_clip`] to `rect`, rounded by `radii`.
    pub fn push_clip(&mut self, rect: Rect, radii: [f32; 4]) {
        self.primitives.push(Primitive::PushClip(rect, radii));
    }

    pub fn pop_clip(&mut self) {
        self.primitives.push(Primitive::PopClip);
    }
}
