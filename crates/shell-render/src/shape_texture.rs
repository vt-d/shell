//! The GPU copy of outline profiles: one row of [`PROFILE_SAMPLES`] radii per
//! [`Outline`], computed and uploaded the first time it is drawn.

use std::{any::TypeId, collections::HashMap};

use crate::outline::{Outline, PROFILE_SAMPLES};

/// Rows the texture starts with; it doubles whenever it fills up.
const INITIAL_ROWS: u32 = 64;

struct Row {
    index: u32,
    /// The profile's largest radius, for sizing the quad that covers it.
    extent: f32,
}

pub(super) struct ShapeTexture {
    texture: wgpu::Texture,
    rows: HashMap<(TypeId, usize), Row>,
    /// Every uploaded profile, by row, to copy into a larger texture.
    profiles: Vec<Vec<f32>>,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
}

impl ShapeTexture {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shape profiles"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let (texture, bind_group) = create(device, &bind_group_layout, INITIAL_ROWS);
        Self { texture, rows: HashMap::new(), profiles: Vec::new(), bind_group_layout, bind_group }
    }

    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// Makes sure `outline`'s profile is on the GPU; returns its row and largest radius
    /// (as a fraction of the shape's size).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        outline: Outline,
    ) -> (u32, f32) {
        if let Some(row) = self.rows.get(&outline.key()) {
            return (row.index, row.extent);
        }

        let mut profile = outline.profile();
        assert_eq!(profile.len(), PROFILE_SAMPLES, "outline profiles have PROFILE_SAMPLES radii");
        profile.iter_mut().for_each(|r| *r = r.max(0.0));
        let index = self.profiles.len() as u32;
        if index == self.texture.height() {
            self.grow(device, queue);
        }
        upload(queue, &self.texture, index, &profile);
        let extent = profile.iter().copied().fold(0.0, f32::max);
        self.profiles.push(profile);
        self.rows.insert(outline.key(), Row { index, extent });
        (index, extent)
    }

    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let rows = (self.texture.height() * 2).min(device.limits().max_texture_dimension_2d);
        assert!(rows > self.texture.height(), "too many distinct outlines");
        (self.texture, self.bind_group) = create(device, &self.bind_group_layout, rows);
        for (index, profile) in self.profiles.iter().enumerate() {
            upload(queue, &self.texture, index as u32, profile);
        }
    }
}

fn create(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    rows: u32,
) -> (wgpu::Texture, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shape profiles"),
        size: wgpu::Extent3d {
            width: PROFILE_SAMPLES as u32,
            height: rows,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shape profiles"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&view),
        }],
    });
    (texture, bind_group)
}

fn upload(queue: &wgpu::Queue, texture: &wgpu::Texture, row: u32, profile: &[f32]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: 0, y: row, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(profile),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(PROFILE_SAMPLES as u32 * 4),
            rows_per_image: None,
        },
        wgpu::Extent3d { width: PROFILE_SAMPLES as u32, height: 1, depth_or_array_layers: 1 },
    );
}
