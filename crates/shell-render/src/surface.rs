//! Per-surface swapchain and GPU buffers.

use std::ptr::NonNull;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use wayland_client::{Connection, Proxy, protocol::wl_surface::WlSurface};

use super::pipeline::{Globals, Instance};

const INITIAL_INSTANCE_CAPACITY: usize = 64;

/// A wgpu swapchain bound to a `wl_surface`.
///
/// Must be dropped before the `wl_surface` it was created from is destroyed.
pub struct RenderSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    configured: bool,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    instance_capacity: usize,
}

impl RenderSurface {
    pub(super) fn new(
        surface: wgpu::Surface<'static>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let mut config =
            surface.get_default_config(adapter, 1, 1).expect("surface not supported by adapter");
        let caps = surface.get_capabilities(adapter);
        // Colors are blended in linear light; an sRGB target encodes them on write.
        if let Some(&srgb) = caps.formats.iter().find(|f| f.is_srgb()) {
            config.format = srgb;
        }
        // Lets windows have transparent regions (rounded or floating bars, popups).
        if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
        }
        // Frame pacing comes from wl_surface.frame callbacks, not from the swapchain. Mailbox
        // never blocks the event loop inside acquire/present; FIFO is the guaranteed fallback.
        config.present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });

        Self {
            surface,
            config,
            configured: false,
            globals,
            bind_group,
            instances: create_instance_buffer(device, INITIAL_INSTANCE_CAPACITY),
            instance_capacity: INITIAL_INSTANCE_CAPACITY,
        }
    }

    pub(super) fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub(super) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    pub(super) fn instances(&self) -> &wgpu::Buffer {
        &self.instances
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.configured && self.config.width == width && self.config.height == height {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(device, &self.config);
        self.configured = true;
    }

    /// Uploads this frame's uniforms and instances, growing the instance buffer if needed.
    pub(super) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[Instance],
    ) {
        let globals = Globals {
            viewport: [self.config.width as f32, self.config.height as f32],
            _pad: [0.0; 2],
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instances = create_instance_buffer(device, self.instance_capacity);
        }
        if !instances.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(instances));
        }
    }

    /// Acquires the next swapchain image, or `None` if the surface can't be drawn to now.
    pub(super) fn acquire(&mut self, device: &wgpu::Device) -> Option<wgpu::SurfaceTexture> {
        if !self.configured {
            return None;
        }
        use wgpu::CurrentSurfaceTexture as T;
        match self.surface.get_current_texture() {
            T::Success(frame) | T::Suboptimal(frame) => Some(frame),
            T::Outdated | T::Lost => {
                // Recreate the swapchain and retry once rather than dropping the frame.
                self.surface.configure(device, &self.config);
                match self.surface.get_current_texture() {
                    T::Success(frame) | T::Suboptimal(frame) => Some(frame),
                    _ => None,
                }
            }
            T::Timeout | T::Occluded | T::Validation => None,
        }
    }
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: (capacity * size_of::<Instance>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// # Safety
/// The returned surface must be dropped before `wl_surface` is destroyed, and the display
/// behind `conn` must outlive it (the [`super::Renderer`] holds a `Connection` for this).
pub(super) unsafe fn create_wgpu_surface(
    instance: &wgpu::Instance,
    conn: &Connection,
    wl_surface: &WlSurface,
) -> wgpu::Surface<'static> {
    let display = NonNull::new(conn.backend().display_ptr().cast()).expect("null wl_display");
    let surface = NonNull::new(wl_surface.id().as_ptr().cast()).expect("null wl_surface");
    unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display))),
            raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(surface)),
        })
    }
    .expect("failed to create wgpu surface")
}
