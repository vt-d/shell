//! GPU rendering on top of wgpu.
//!
//! A single [`Renderer`] owns the GPU context (instance, adapter, device and queue), the
//! system [`Fonts`], the glyph atlas and the [`Outline`]s drawn so far, and is shared by
//! every [`RenderSurface`], each of which wraps one Wayland surface. What to draw is
//! described by a [`Scene`]: rounded rectangles, filled (and morphing) outlines, and text.

mod atlas;
mod image;
mod outline;
mod pipeline;
mod scene;
mod shape_texture;
mod surface;
mod symbol;
mod text;

pub use cosmic_text;
use wayland_client::{Connection, protocol::wl_surface::WlSurface};

use atlas::{AtlasFull, AtlasKey, GlyphAtlas};
pub use image::{Image, ImageFile};
pub use outline::{Outline, OutlineSource, PROFILE_SAMPLES};
use pipeline::{Instance, Pipeline};
pub use scene::{Color, Morph, Primitive, Quad, Rect, Scene, TextRun, Wave};
use shape_texture::ShapeTexture;
pub use surface::RenderSurface;
use symbol::Symbols;
pub use symbol::{Axis, MAX_AXES, Symbol, SymbolFont};
pub use text::Fonts;

pub struct Renderer {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: Pipeline,
    atlas: GlyphAtlas,
    shapes: ShapeTexture,
    fonts: Fonts,
    symbols: Symbols,
    // Holding the connection keeps the wl_display alive for every surface made from it.
    conn: Connection,
}

impl Renderer {
    /// Creates the GPU context, picking an adapter that can present to `wl_surface`,
    /// and returns it together with that surface's render target.
    pub fn new(conn: &Connection, wl_surface: &WlSurface) -> (Self, RenderSurface) {
        let instance = wgpu::Instance::default();
        let surface = unsafe { surface::create_wgpu_surface(&instance, conn, wl_surface) };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no suitable GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("failed to create device");

        let atlas = GlyphAtlas::new(&device);
        let shapes = ShapeTexture::new(&device);
        let pipeline =
            Pipeline::new(&device, atlas.bind_group_layout(), shapes.bind_group_layout());
        let mut renderer = Self {
            instance,
            adapter,
            device,
            queue,
            pipeline,
            atlas,
            shapes,
            fonts: Fonts::new(),
            symbols: Symbols::default(),
            conn: conn.clone(),
        };
        let target = renderer.wrap_surface(surface);
        (renderer, target)
    }

    /// Creates a render target for another `wl_surface` using the existing GPU context.
    pub fn create_surface(&mut self, wl_surface: &WlSurface) -> RenderSurface {
        let surface =
            unsafe { surface::create_wgpu_surface(&self.instance, &self.conn, wl_surface) };
        self.wrap_surface(surface)
    }

    fn wrap_surface(&mut self, surface: wgpu::Surface<'static>) -> RenderSurface {
        let target =
            RenderSurface::new(surface, &self.adapter, &self.device, &self.pipeline.globals_layout);
        self.pipeline.prepare(&self.device, target.format());
        target
    }

    /// The fonts text must be shaped with to be drawn by this renderer.
    pub fn fonts(&mut self) -> &mut Fonts {
        &mut self.fonts
    }

    /// Resizes `target`'s swapchain to `width` x `height` physical pixels.
    pub fn resize(&self, target: &mut RenderSurface, width: u32, height: u32) {
        target.resize(&self.device, width, height);
    }

    /// Draws `scene` (in logical pixels, multiplied by `scale`) and presents it.
    ///
    /// `before_present` runs once a frame has been acquired, right before the present that
    /// commits the `wl_surface`. Use it to attach per-commit state such as a frame callback.
    /// Returns `false` if nothing was presented (and so `before_present` never ran).
    pub fn render(
        &mut self,
        target: &mut RenderSurface,
        scene: &Scene,
        scale: f32,
        before_present: impl FnOnce(),
    ) -> bool {
        let Some(frame) = target.acquire(&self.device) else {
            return false;
        };

        let instances = self.build_instances(scene, scale);
        target.upload(&self.device, &self.queue, &instances);

        let [r, g, b, a] = scene.clear.to_linear_premultiplied().map(f64::from);
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if !instances.is_empty() {
                pass.set_pipeline(self.pipeline.get(target.format()));
                pass.set_bind_group(0, target.bind_group(), &[]);
                pass.set_bind_group(1, self.atlas.bind_group(), &[]);
                pass.set_bind_group(2, self.shapes.bind_group(), &[]);
                pass.set_vertex_buffer(0, target.instances().slice(..));
                pass.draw(0..4, 0..instances.len() as u32);
            }
        }
        self.queue.submit([encoder.finish()]);
        before_present();
        self.queue.present(frame);
        true
    }

    fn build_instances(&mut self, scene: &Scene, scale: f32) -> Vec<Instance> {
        let mut instances = Vec::with_capacity(scene.primitives.len());
        if self.push_instances(scene, scale, false, &mut instances).is_err() {
            // The atlas filled up. Start over with only this frame's glyphs; any that still
            // don't fit are left out.
            self.atlas.clear();
            instances.clear();
            let _ = self.push_instances(scene, scale, true, &mut instances);
        }
        instances
    }

    fn push_instances(
        &mut self,
        scene: &Scene,
        scale: f32,
        skip_unfit: bool,
        out: &mut Vec<Instance>,
    ) -> Result<(), AtlasFull> {
        // The clips in force, innermost last: `[left, top, right, bottom]` and corner radii,
        // in physical pixels.
        let mut clips: Vec<([f32; 4], [f32; 4])> = Vec::new();
        for primitive in &scene.primitives {
            let first = out.len();
            match primitive {
                Primitive::PushClip(rect, radii) => {
                    let mut clip = [
                        rect.x * scale,
                        rect.y * scale,
                        (rect.x + rect.width) * scale,
                        (rect.y + rect.height) * scale,
                    ];
                    if let Some((outer, _)) = clips.last() {
                        clip = [
                            clip[0].max(outer[0]),
                            clip[1].max(outer[1]),
                            clip[2].min(outer[2]).max(clip[0].max(outer[0])),
                            clip[3].min(outer[3]).max(clip[1].max(outer[1])),
                        ];
                    }
                    clips.push((clip, radii.map(|r| r * scale)));
                }
                Primitive::PopClip => {
                    clips.pop();
                }
                Primitive::Wave(wave) => out.push(Instance::wave(wave, scale)),
                Primitive::Quad(quad) => out.push(Instance::quad(quad, scale)),
                Primitive::Morph(morph) => {
                    let (from, from_extent) =
                        self.shapes.prepare(&self.device, &self.queue, morph.from);
                    let (to, to_extent) = self.shapes.prepare(&self.device, &self.queue, morph.to);
                    out.push(Instance::morph(morph, [from, to], from_extent.max(to_extent), scale));
                }
                Primitive::Symbol(symbol) => {
                    let Some(placed) = self.symbols.place(symbol, scale) else {
                        continue;
                    };
                    let symbols = &mut self.symbols;
                    let cached = match self.atlas.get(
                        &self.device,
                        &self.queue,
                        AtlasKey::Symbol(placed.key),
                        || symbols.rasterize(symbol.font, placed.key),
                    ) {
                        Ok(cached) => cached,
                        Err(AtlasFull) if skip_unfit => None,
                        Err(full) => return Err(full),
                    };
                    let Some(image) = cached else { continue };
                    let [x, y] = placed.origin;
                    out.push(Instance::glyph(
                        x + image.offset[0],
                        y + image.offset[1],
                        &image,
                        symbol.color,
                    ));
                }
                Primitive::Image(image) => {
                    if image.opacity <= 0.0 {
                        continue;
                    }
                    let placed = image::place(image, scale);
                    let cached = match self.atlas.get(
                        &self.device,
                        &self.queue,
                        AtlasKey::Image(placed.key),
                        || image::rasterize(placed.key),
                    ) {
                        Ok(cached) => cached,
                        Err(AtlasFull) if skip_unfit => None,
                        Err(full) => return Err(full),
                    };
                    let Some(glyph) = cached else { continue };
                    let [x, y] = placed.origin;
                    out.push(Instance::glyph(
                        x + glyph.offset[0],
                        y + glyph.offset[1],
                        &glyph,
                        image.color(),
                    ));
                }
                Primitive::Text(text) => {
                    for run in text.buffer.layout_runs() {
                        let line_origin =
                            (text.origin[0] * scale, (text.origin[1] + run.line_y) * scale);
                        for glyph in run.glyphs {
                            // Physical size and position, snapped to the pixel grid
                            // vertically and to quarter pixels horizontally.
                            let physical = glyph.physical(line_origin, scale);
                            let fonts = &mut self.fonts;
                            let cached = match self.atlas.get(
                                &self.device,
                                &self.queue,
                                AtlasKey::Text(physical.cache_key),
                                || fonts.rasterize(physical.cache_key),
                            ) {
                                Ok(cached) => cached,
                                Err(AtlasFull) if skip_unfit => None,
                                Err(full) => return Err(full),
                            };
                            let Some(image) = cached else { continue };
                            let color = glyph.color_opt.map_or(text.color, |c| {
                                let [r, g, b, a] = c.as_rgba().map(|v| v as f32 / 255.0);
                                Color::rgba(r, g, b, a)
                            });
                            out.push(Instance::glyph(
                                physical.x + image.offset[0],
                                physical.y + image.offset[1],
                                &image,
                                color,
                            ));
                        }
                    }
                }
            }
            if let Some(&(clip, radii)) = clips.last() {
                for instance in &mut out[first..] {
                    *instance = instance.clipped(clip, radii);
                }
            }
        }
        Ok(())
    }
}
