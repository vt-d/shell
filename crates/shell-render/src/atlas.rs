//! The glyph atlas: rasterized glyphs and symbols packed into GPU textures, shared by
//! every surface.
//!
//! There are two pages: an 8-bit coverage mask for ordinary glyphs (tinted by the text
//! color in the shader) and an sRGB RGBA page for color glyphs such as emoji. Each page
//! starts small and doubles when full, keeping what it holds. A page that can't grow any
//! further makes [`GlyphAtlas::get`] report [`AtlasFull`]; the renderer then clears the
//! atlas and rebuilds the frame from scratch, so only glyphs still in use come back.

use std::collections::HashMap;

use cosmic_text::{CacheKey, SwashContent, SwashImage};
use etagere::{BucketedAtlasAllocator, size2};

use crate::{image::ImageKey, symbol::SymbolKey};

const INITIAL_SIZE: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GlyphKind {
    Mask,
    Color,
}

/// Where a cached glyph lives, and how to place it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Glyph {
    pub kind: GlyphKind,
    /// Top-left texel of the image in its page.
    pub uv: [u32; 2],
    pub size: [u32; 2],
    /// From the glyph's pen position (on the baseline) to the image's top-left corner,
    /// in physical pixels, y down.
    pub offset: [i32; 2],
}

/// What an atlas image is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum AtlasKey {
    Text(CacheKey),
    Symbol(SymbolKey),
    Image(ImageKey),
}

/// The atlas has no room for another glyph, even at its maximum size.
#[derive(Debug)]
pub(super) struct AtlasFull;

pub(super) struct GlyphAtlas {
    mask: Page,
    color: Page,
    /// `None` for glyphs with no image (spaces, missing glyphs).
    glyphs: HashMap<AtlasKey, Option<Glyph>>,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    max_size: u32,
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glyph atlas"),
            entries: &[texture_entry(0), texture_entry(1)],
        });
        let mask = Page::new(device, wgpu::TextureFormat::R8Unorm, INITIAL_SIZE);
        // Emoji bitmaps are sRGB-encoded; sampling an sRGB view decodes them to linear.
        let color = Page::new(device, wgpu::TextureFormat::Rgba8UnormSrgb, INITIAL_SIZE);
        let bind_group = create_bind_group(device, &bind_group_layout, &mask, &color);
        Self {
            mask,
            color,
            glyphs: HashMap::new(),
            bind_group_layout,
            bind_group,
            max_size: device.limits().max_texture_dimension_2d,
        }
    }

    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// Looks up an image, calling `rasterize` to make it on first use.
    pub fn get(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: AtlasKey,
        rasterize: impl FnOnce() -> Option<SwashImage>,
    ) -> Result<Option<Glyph>, AtlasFull> {
        if let Some(&glyph) = self.glyphs.get(&key) {
            return Ok(glyph);
        }
        let glyph = match rasterize() {
            Some(image) => self.insert(device, queue, &image)?,
            None => None,
        };
        self.glyphs.insert(key, glyph);
        Ok(glyph)
    }

    fn insert(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &SwashImage,
    ) -> Result<Option<Glyph>, AtlasFull> {
        let (width, height) = (image.placement.width, image.placement.height);
        if width == 0 || height == 0 {
            return Ok(None);
        }
        let (kind, page) = match image.content {
            SwashContent::Mask => (GlyphKind::Mask, &mut self.mask),
            SwashContent::Color => (GlyphKind::Color, &mut self.color),
            // Only requested for LCD subpixel rendering, which we don't ask for.
            SwashContent::SubpixelMask => return Ok(None),
        };

        let mut grew = false;
        let origin = loop {
            if let Some(allocation) = page.allocator.allocate(size2(width as i32, height as i32)) {
                break allocation.rectangle.min;
            }
            if page.size >= self.max_size {
                // A glyph bigger than an empty atlas can never be drawn; don't retry it.
                return if page.allocator.is_empty() { Ok(None) } else { Err(AtlasFull) };
            }
            page.grow(device, queue, (page.size * 2).min(self.max_size));
            grew = true;
        };
        if grew {
            self.bind_group =
                create_bind_group(device, &self.bind_group_layout, &self.mask, &self.color);
        }

        let page = if kind == GlyphKind::Mask { &self.mask } else { &self.color };
        let uv = [origin.x as u32, origin.y as u32];
        page.write(queue, uv, [width, height], &image.data);
        Ok(Some(Glyph {
            kind,
            uv,
            size: [width, height],
            offset: [image.placement.left, -image.placement.top],
        }))
    }

    /// Forgets every glyph so their space can be reused.
    pub fn clear(&mut self) {
        self.glyphs.clear();
        self.mask.allocator.clear();
        self.color.allocator.clear();
    }
}

struct Page {
    format: wgpu::TextureFormat,
    allocator: BucketedAtlasAllocator,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: u32,
}

impl Page {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: u32) -> Self {
        let (texture, view) = create_texture(device, format, size);
        let allocator = BucketedAtlasAllocator::new(size2(size as i32, size as i32));
        Self { format, allocator, texture, view, size }
    }

    /// Replaces the texture with a larger one holding the same glyphs at the same places.
    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, size: u32) {
        let (texture, view) = create_texture(device, self.format, size);
        let mut encoder = device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("atlas grow") });
        // Pending write_texture calls to the old texture run before this submission.
        encoder.copy_texture_to_texture(
            self.texture.as_image_copy(),
            texture.as_image_copy(),
            wgpu::Extent3d { width: self.size, height: self.size, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);
        self.allocator.grow(size2(size as i32, size as i32));
        self.texture = texture;
        self.view = view;
        self.size = size;
    }

    fn write(&self, queue: &wgpu::Queue, [x, y]: [u32; 2], [width, height]: [u32; 2], data: &[u8]) {
        let bytes_per_pixel = self.format.block_copy_size(None).expect("uncompressed format");
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * bytes_per_pixel),
                rows_per_image: None,
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
    }
}

fn create_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glyph atlas"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    mask: &Page,
    color: &Page,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("glyph atlas"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&mask.view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&color.view),
            },
        ],
    })
}
