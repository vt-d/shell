//! Images, such as app icons and album art: PNG, JPEG and SVG files, and SVGs built into
//! the binary. They're
//! rasterized at the exact size they're drawn at (SVGs rendered, PNGs area-averaged, so
//! large icons scale down cleanly) and cached in the glyph atlas: in full color, or, for
//! [tinted](Image::tint) single-color icons, as a coverage mask like text.

use std::{
    collections::HashMap,
    fmt, fs,
    io::Cursor,
    path::Path,
    sync::{Arc, LazyLock, Mutex},
};

use cosmic_text::{SwashContent, SwashImage};
use resvg::{tiny_skia, usvg};

use crate::scene::Color;

/// An image file, or an SVG built into the binary. Cheap to copy and compare: sources
/// are interned.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageFile(u32);

#[derive(Clone)]
enum Source {
    File(Arc<Path>),
    /// SVG data, identified by its address.
    Svg(&'static [u8]),
}

#[derive(Default)]
struct Sources {
    all: Vec<Source>,
    files: HashMap<Arc<Path>, u32>,
    svgs: HashMap<usize, u32>,
}

static SOURCES: LazyLock<Mutex<Sources>> = LazyLock::new(Mutex::default);

impl ImageFile {
    /// A PNG, JPEG or SVG file, by path. SVGs are told by their extension, the others by
    /// their contents.
    pub fn new(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let mut sources = SOURCES.lock().unwrap();
        if let Some(&id) = sources.files.get(path) {
            return Self(id);
        }
        let id = sources.all.len() as u32;
        let path: Arc<Path> = path.into();
        sources.all.push(Source::File(path.clone()));
        sources.files.insert(path, id);
        Self(id)
    }

    /// An SVG built into the binary, typically with `include_bytes!`.
    pub fn svg(data: &'static [u8]) -> Self {
        let mut sources = SOURCES.lock().unwrap();
        let address = data.as_ptr() as usize;
        if let Some(&id) = sources.svgs.get(&address) {
            return Self(id);
        }
        let id = sources.all.len() as u32;
        sources.all.push(Source::Svg(data));
        sources.svgs.insert(address, id);
        Self(id)
    }

    fn source(self) -> Source {
        SOURCES.lock().unwrap().all[self.0 as usize].clone()
    }
}

impl fmt::Debug for ImageFile {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.source() {
            Source::File(path) => f.debug_tuple("ImageFile").field(&path).finish(),
            Source::Svg(data) => write!(f, "ImageFile(<{} bytes of SVG>)", data.len()),
        }
    }
}

/// An image, fit (keeping its aspect ratio) into a `size` by `size` square centered on
/// `center`, or [covering](Image::cover) it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Image {
    pub file: ImageFile,
    pub center: [f32; 2],
    /// Logical pixels.
    pub size: f32,
    /// 0 is invisible, 1 opaque.
    pub opacity: f32,
    /// Draws only the image's shape (its alpha), in this color, as Android tints its
    /// icons. Otherwise the image keeps its own colors.
    pub tint: Option<Color>,
    /// Fills the square, cropping what of the image doesn't fit, rather than fitting
    /// all of the image inside it. (SVGs are always fit.)
    pub cover: bool,
}

impl Image {
    /// What the instance is drawn with: the tint, or for a color image white, whose
    /// alpha alone counts.
    pub(crate) fn color(&self) -> Color {
        let opacity = self.opacity.clamp(0.0, 1.0);
        match self.tint {
            Some(tint) => tint.with_alpha(tint.a * opacity),
            None => Color::rgba(1.0, 1.0, 1.0, opacity),
        }
    }
}

/// What identifies one rasterized image in the glyph atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ImageKey {
    file: ImageFile,
    /// Physical pixels.
    size: u32,
    /// Rasterized as a coverage mask, for tinting.
    mask: bool,
    /// Cropped to fill the square.
    cover: bool,
}

/// Where an image goes, in physical pixels, and its atlas key.
pub(crate) struct Placed {
    pub key: ImageKey,
    pub origin: [i32; 2],
}

pub(crate) fn place(image: &Image, scale: f32) -> Placed {
    let size = (image.size * scale).round().max(1.0);
    let origin = image.center.map(|c| (c * scale - size / 2.0).round() as i32);
    let key = ImageKey {
        file: image.file,
        size: size as u32,
        mask: image.tint.is_some(),
        cover: image.cover,
    };
    Placed { key, origin }
}

/// Reads and rasterizes an image, as straight (not premultiplied) sRGB RGBA as the color
/// atlas takes or as coverage, placed relative to the square's top-left corner.
pub(crate) fn rasterize(key: ImageKey) -> Option<SwashImage> {
    let source = key.file.source();
    let result = match &source {
        Source::Svg(data) => render_svg(data, key.size),
        Source::File(path) => match path.extension().and_then(|ext| ext.to_str()) {
            Some("svg") => {
                fs::read(path).map_err(Error::from).and_then(|data| render_svg(&data, key.size))
            }
            _ => decode(path).map(|pixels| {
                if key.cover { pixels.cropped_square().fit(key.size) } else { pixels.fit(key.size) }
            }),
        },
    };
    let pixels = match result {
        Ok(pixels) => pixels,
        Err(err) => {
            eprintln!("can't load {:?}: {err}", key.file);
            return None;
        }
    };
    let mut image = SwashImage::new();
    image.placement.left = pixels.left as i32;
    image.placement.top = -(pixels.top as i32);
    image.placement.width = pixels.width;
    image.placement.height = pixels.height;
    if key.mask {
        image.content = SwashContent::Mask;
        image.data = pixels.into_coverage();
    } else {
        image.content = SwashContent::Color;
        image.data = pixels.into_straight_rgba();
    }
    Some(image)
}

type Error = Box<dyn std::error::Error>;

/// Premultiplied RGBA, 0–1, placed at `left`, `top` in the square drawn into.
struct Pixels {
    width: u32,
    height: u32,
    left: u32,
    top: u32,
    data: Vec<[f32; 4]>,
}

impl Pixels {
    fn from_premultiplied(width: u32, height: u32, rgba: &[u8]) -> Self {
        let data = rgba.as_chunks::<4>().0.iter().map(|p| p.map(|c| c as f32 / 255.0)).collect();
        Self { width, height, left: 0, top: 0, data }
    }

    /// The largest square from its middle.
    fn cropped_square(self) -> Self {
        let side = self.width.min(self.height);
        let (x0, y0) = ((self.width - side) / 2, (self.height - side) / 2);
        let data = (0..side)
            .flat_map(|y| {
                let start = ((y0 + y) * self.width + x0) as usize;
                self.data[start..start + side as usize].iter().copied()
            })
            .collect();
        Self { width: side, height: side, left: 0, top: 0, data }
    }

    /// Scales to fit a `size` by `size` square, keeping the aspect ratio, by averaging
    /// the source pixels each destination pixel covers.
    fn fit(self, size: u32) -> Self {
        let scale = size as f32 / self.width.max(self.height) as f32;
        let width = ((self.width as f32 * scale).round() as u32).clamp(1, size);
        let height = ((self.height as f32 * scale).round() as u32).clamp(1, size);
        let columns: Vec<_> = (0..width).map(|x| weights(self.width, width, x)).collect();
        let rows: Vec<_> = (0..height).map(|y| weights(self.height, height, y)).collect();
        let mut data = Vec::with_capacity((width * height) as usize);
        for row in &rows {
            for column in &columns {
                let mut sum = [0.0; 4];
                for &(sy, wy) in row {
                    for &(sx, wx) in column {
                        let pixel = self.data[(sy * self.width + sx) as usize];
                        for (s, p) in sum.iter_mut().zip(pixel) {
                            *s += p * wx * wy;
                        }
                    }
                }
                data.push(sum);
            }
        }
        Self { width, height, left: (size - width) / 2, top: (size - height) / 2, data }
    }

    /// Alpha alone: how much of each pixel the image covers.
    fn into_coverage(self) -> Vec<u8> {
        self.data.iter().map(|[.., a]| (a.clamp(0.0, 1.0) * 255.0).round() as u8).collect()
    }

    fn into_straight_rgba(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len() * 4);
        for [r, g, b, a] in self.data {
            let unpremultiply = if a > 0.0 { 1.0 / a } else { 0.0 };
            for c in [r * unpremultiply, g * unpremultiply, b * unpremultiply, a] {
                out.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        out
    }
}

/// The source pixels destination pixel `d` covers along one axis, and how much of it
/// each covers (summing to 1).
fn weights(source: u32, destination: u32, d: u32) -> Vec<(u32, f32)> {
    let ratio = source as f32 / destination as f32;
    let (start, end) = (d as f32 * ratio, (d + 1) as f32 * ratio);
    let first = start.floor() as u32;
    let last = (end.ceil() as u32).min(source).max(first + 1);
    (first..last)
        .map(|s| (s, (end.min(s as f32 + 1.0) - start.max(s as f32)).max(0.0) / (end - start)))
        .collect()
}

/// Decodes a PNG or JPEG file, whichever its contents say it is.
fn decode(path: &Path) -> Result<Pixels, Error> {
    let data = fs::read(path)?;
    if data.starts_with(&[0xff, 0xd8]) { decode_jpeg(&data) } else { decode_png(&data) }
}

fn decode_jpeg(data: &[u8]) -> Result<Pixels, Error> {
    use zune_jpeg::zune_core::{colorspace::ColorSpace, options::DecoderOptions};
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(data, options);
    let bytes = decoder.decode().map_err(|err| format!("{err:?}"))?;
    let (width, height) = decoder.dimensions().ok_or("no dimensions")?;
    let data = bytes
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&[r, g, b]| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        .collect();
    Ok(Pixels { width: width as u32, height: height as u32, left: 0, top: 0, data })
}

fn decode_png(data: &[u8]) -> Result<Pixels, Error> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("image too large")?];
    let frame = reader.next_frame(&mut buffer)?;
    let bytes = &buffer[..frame.buffer_size()];
    let rgba: Vec<[u8; 4]> = match frame.color_type {
        png::ColorType::Rgba => bytes.as_chunks::<4>().0.to_vec(),
        png::ColorType::Rgb => {
            bytes.as_chunks::<3>().0.iter().map(|&[r, g, b]| [r, g, b, 255]).collect()
        }
        png::ColorType::GrayscaleAlpha => {
            bytes.as_chunks::<2>().0.iter().map(|&[v, a]| [v, v, v, a]).collect()
        }
        png::ColorType::Grayscale => bytes.iter().map(|&v| [v, v, v, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed color left unexpanded".into()),
    };
    // Premultiplied, so averaging doesn't bleed transparent pixels' colors in.
    let data = rgba
        .iter()
        .map(|&[r, g, b, a]| {
            let a = a as f32 / 255.0;
            let premultiply = |c: u8| c as f32 / 255.0 * a;
            [premultiply(r), premultiply(g), premultiply(b), a]
        })
        .collect();
    Ok(Pixels { width: frame.width, height: frame.height, left: 0, top: 0, data })
}

fn render_svg(data: &[u8], size: u32) -> Result<Pixels, Error> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default())?;
    let (w, h) = (tree.size().width(), tree.size().height());
    let scale = size as f32 / w.max(h);
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("empty image")?;
    let transform = tiny_skia::Transform::from_scale(scale, scale)
        .post_translate((size as f32 - w * scale) / 2.0, (size as f32 - h * scale) / 2.0);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(Pixels::from_premultiplied(size, size, pixmap.data()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_cover_each_destination_pixel_once() {
        for (source, destination) in [(256, 40), (40, 40), (16, 40), (7, 3)] {
            for d in 0..destination {
                let total: f32 = weights(source, destination, d).iter().map(|(_, w)| w).sum();
                assert!((total - 1.0).abs() < 1e-4, "{source}→{destination} at {d}: {total}");
            }
        }
    }

    #[test]
    fn fitting_keeps_aspect_ratio_and_centers() {
        let wide = Pixels::from_premultiplied(4, 2, &[255; 4 * 2 * 4]);
        let fit = wide.fit(2);
        assert_eq!((fit.width, fit.height, fit.left, fit.top), (2, 1, 0, 0));
        assert!(fit.data.iter().all(|p| p.iter().all(|&c| (c - 1.0).abs() < 1e-5)));
    }
}
