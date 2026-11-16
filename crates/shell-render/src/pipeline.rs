//! The one render pipeline: instanced rounded rectangles, shapes and glyphs.
//!
//! Every primitive is an [`Instance`] of the same 4-vertex strip, so a whole scene is
//! a single draw call and quads and text interleave in exactly the order painted.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};

use super::{
    atlas::{Glyph, GlyphKind},
    scene::{Color, Morph, Quad, Wave},
};

/// Per-surface uniforms. Matches `Globals` in `shader.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct Globals {
    pub viewport: [f32; 2],
    pub _pad: [f32; 2],
}

// Matches the `KIND_*` constants in `shader.wgsl`.
const KIND_QUAD: u32 = 0;
const KIND_MASK_GLYPH: u32 = 1;
const KIND_COLOR_GLYPH: u32 = 2;
const KIND_MORPH: u32 = 3;
const KIND_WAVE: u32 = 4;

/// A clip that cuts nothing off: `[left, top, right, bottom]`.
const NO_CLIP: [f32; 4] = [-1e9, -1e9, 1e9, 1e9];

/// Per-primitive vertex data, in physical pixels. Matches `Instance` in `shader.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct Instance {
    rect: [f32; 4],
    /// Linear, premultiplied.
    color: [f32; 4],
    /// Depends on `kind`. Quads: corner radii (clockwise from top-left). Glyphs: atlas texel of the rect's
    /// top-left corner. Morphs: profile rows of both shapes, progress, rotation.
    /// Waves: amplitude, wavelength, phase, stroke width.
    params: [f32; 4],
    /// What it is clipped to: `[left, top, right, bottom]`, and that rectangle's corner
    /// radii, clockwise from top-left.
    clip: [f32; 4],
    clip_radii: [f32; 4],
    kind: u32,
    /// Morphs: the shapes' size.
    size: f32,
    _pad: [u32; 2],
}

impl Instance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
        0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4,
        5 => Uint32, 6 => Float32
    ];

    /// Clips the instance to a rounded rectangle, in physical pixels.
    pub fn clipped(mut self, clip: [f32; 4], radii: [f32; 4]) -> Self {
        self.clip = clip;
        self.clip_radii = radii;
        self
    }

    /// Converts a logical-pixel wave to physical pixels.
    pub fn wave(wave: &Wave, scale: f32) -> Self {
        let r = wave.rect;
        Self {
            rect: [r.x * scale, r.y * scale, r.width * scale, r.height * scale],
            color: wave.color.to_linear_premultiplied(),
            params: [
                wave.amplitude * scale,
                wave.wavelength * scale,
                wave.phase,
                wave.thickness * scale,
            ],
            kind: KIND_WAVE,
            size: 0.0,
            _pad: [0; 2],
            clip: NO_CLIP,
            clip_radii: [0.0; 4],
        }
    }

    /// Converts a logical-pixel quad to physical pixels.
    pub fn quad(quad: &Quad, scale: f32) -> Self {
        let r = quad.rect;
        Self {
            rect: [r.x * scale, r.y * scale, r.width * scale, r.height * scale],
            color: quad.color.to_linear_premultiplied(),
            params: quad.corner_radii.map(|r| r * scale),
            kind: KIND_QUAD,
            size: 0.0,
            _pad: [0; 2],
            clip: NO_CLIP,
            clip_radii: [0.0; 4],
        }
    }

    /// Converts a logical-pixel morph to physical pixels. `rows` are the shapes' rows in
    /// the profile texture; `extent` is the largest radius either reaches, as a fraction
    /// of the size.
    pub fn morph(morph: &Morph, rows: [u32; 2], extent: f32, scale: f32) -> Self {
        let size = morph.size * scale;
        // The quad covers the outline at any rotation, plus a pixel for anti-aliasing.
        let half = size * extent + 1.0;
        let [x, y] = morph.center.map(|c| c * scale);
        Self {
            rect: [x - half, y - half, 2.0 * half, 2.0 * half],
            color: morph.color.to_linear_premultiplied(),
            params: [rows[0] as f32, rows[1] as f32, morph.progress, morph.rotation],
            kind: KIND_MORPH,
            size,
            _pad: [0; 2],
            clip: NO_CLIP,
            clip_radii: [0.0; 4],
        }
    }

    /// A glyph image whose top-left corner is at physical pixel `(x, y)`. Glyphs are
    /// drawn texel-for-pixel, so the position must be whole pixels.
    pub fn glyph(x: i32, y: i32, glyph: &Glyph, color: Color) -> Self {
        let [w, h] = glyph.size;
        Self {
            rect: [x as f32, y as f32, w as f32, h as f32],
            color: color.to_linear_premultiplied(),
            params: [glyph.uv[0] as f32, glyph.uv[1] as f32, 0.0, 0.0],
            kind: match glyph.kind {
                GlyphKind::Mask => KIND_MASK_GLYPH,
                GlyphKind::Color => KIND_COLOR_GLYPH,
            },
            size: 0.0,
            _pad: [0; 2],
            clip: NO_CLIP,
            clip_radii: [0.0; 4],
        }
    }
}

pub(super) struct Pipeline {
    pub globals_layout: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    /// Surfaces may differ in format, so pipelines are built per format on demand.
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl Pipeline {
    pub fn new(
        device: &wgpu::Device,
        atlas_layout: &wgpu::BindGroupLayout,
        shapes_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            // Group 0 is per surface; 1 and 2 are the renderer-wide glyph atlas and shape
            // profiles.
            bind_group_layouts: &[Some(&globals_layout), Some(atlas_layout), Some(shapes_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        Self { globals_layout, layout, shader, pipelines: HashMap::new() }
    }

    pub fn prepare(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        self.pipelines
            .entry(format)
            .or_insert_with(|| create_pipeline(device, &self.layout, &self.shader, format));
    }

    /// The pipeline for `format`. [`Pipeline::prepare`] must have been called for it.
    pub fn get(&self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        &self.pipelines[&format]
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("scene"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Instance>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &Instance::ATTRIBUTES,
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
