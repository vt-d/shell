// Every primitive is one instance of a 4-vertex triangle strip covering its rect:
// anti-aliased rounded rectangles and Material shapes, and glyphs copied texel-for-pixel
// from the atlas.
// All positions are physical pixels; colors are linear and premultiplied.

struct Globals {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var mask_atlas: texture_2d<f32>;
@group(1) @binding(1) var color_atlas: texture_2d<f32>;
// One row per shape: the outline's distance from the center at evenly spaced angles,
// as a fraction of the shape's size. See `outline.rs`.
@group(2) @binding(0) var shape_profiles: texture_2d<f32>;

const KIND_QUAD: u32 = 0u;
const KIND_MASK_GLYPH: u32 = 1u;
const KIND_COLOR_GLYPH: u32 = 2u;
const KIND_MORPH: u32 = 3u;
const KIND_WAVE: u32 = 4u;

const TAU: f32 = 6.28318530718;
const PROFILE_SAMPLES: i32 = 256; // `outline::PROFILE_SAMPLES`

// See `text_coverage`. 2.2 reproduces the weight of text blended in sRGB space, which
// is what fonts are designed and hinted against; lower values look thinner on dark
// backgrounds and lighter on light ones.
const TEXT_GAMMA: f32 = 2.2;

struct Instance {
    @location(0) rect: vec4<f32>,   // x, y, width, height
    @location(1) color: vec4<f32>,
    // Quads: corner radii, clockwise from top-left. Glyphs: atlas texel at the top-left.
    // Morphs: profile rows of the two shapes, progress, rotation.
    // Waves: amplitude, wavelength, phase, stroke width.
    @location(2) params: vec4<f32>,
    // What it is clipped to: left, top, right, bottom; and that rectangle's corner radii.
    @location(3) clip: vec4<f32>,
    @location(4) clip_radii: vec4<f32>,
    @location(5) kind: u32,
    @location(6) size: f32,         // morphs: the shapes' size
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    // Fragment position relative to the rect's center.
    @location(0) local: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) @interpolate(flat) params: vec4<f32>,
    @location(5) @interpolate(flat) kind: u32,
    @location(6) @interpolate(flat) size: f32,
    @location(7) @interpolate(flat) clip: vec4<f32>,
    @location(8) @interpolate(flat) clip_radii: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32, instance: Instance) -> VertexOutput {
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let size = instance.rect.zw;
    let pixel = instance.rect.xy + corner * size;
    let ndc = vec2<f32>(
        pixel.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - pixel.y / globals.viewport.y * 2.0,
    );

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.local = (corner - 0.5) * size;
    out.half_size = size * 0.5;
    out.color = instance.color;
    out.uv = instance.params.xy + corner * size;
    out.params = instance.params;
    out.kind = instance.kind;
    out.size = instance.size;
    out.clip = instance.clip;
    out.clip_radii = instance.clip_radii;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // The clip, as a rounded rectangle around its own center.
    let clip_half = (in.clip.zw - in.clip.xy) * 0.5;
    let clip_local = in.position.xy - (in.clip.xy + in.clip.zw) * 0.5;
    return shade(in) * rounded_coverage(clip_local, clip_half, in.clip_radii);
}

fn shade(in: VertexOutput) -> vec4<f32> {
    switch in.kind {
        case KIND_MASK_GLYPH: {
            // Pixel centers land mid-texel, so truncation picks exactly the texel under them.
            let coverage = textureLoad(mask_atlas, vec2<i32>(in.uv), 0).r;
            return in.color * text_coverage(coverage, in.color);
        }
        case KIND_COLOR_GLYPH: {
            // Emoji keep their own colors; the text color only contributes its opacity.
            let texel = textureLoad(color_atlas, vec2<i32>(in.uv), 0);
            return vec4<f32>(texel.rgb * texel.a, texel.a) * in.color.a;
        }
        case KIND_MORPH: {
            return in.color * morph_coverage(in.local, in.params, in.size);
        }
        case KIND_WAVE: {
            return in.color * wave_coverage(in.local, in.half_size, in.params);
        }
        default: {
            return in.color * rounded_coverage(in.local, in.half_size, in.params);
        }
    }
}

// A shape's radius at `angle` (radians, clockwise from +x), linearly interpolated
// between its profile's samples.
fn profile_radius(row: i32, angle: f32) -> f32 {
    let position = fract(angle / TAU) * f32(PROFILE_SAMPLES);
    let index = i32(floor(position));
    let a = textureLoad(shape_profiles, vec2<i32>(index % PROFILE_SAMPLES, row), 0).r;
    let b = textureLoad(shape_profiles, vec2<i32>((index + 1) % PROFILE_SAMPLES, row), 0).r;
    return mix(a, b, position - f32(index));
}

// The morph's outline: both shapes' radii at `angle`, blended by progress.
fn morph_radius(params: vec4<f32>, size: f32, angle: f32) -> f32 {
    let start = profile_radius(i32(params.x), angle);
    let end = profile_radius(i32(params.y), angle);
    return mix(start, end, params.z) * size;
}

fn morph_coverage(p: vec2<f32>, params: vec4<f32>, size: f32) -> f32 {
    let distance_to_center = max(length(p), 1e-4);
    let angle = atan2(p.y, p.x) - params.w;
    let edge = morph_radius(params, size, angle);
    // r - edge(θ) is a distance only along rays from the center. Dividing by the
    // gradient's length makes it one near the outline, where anti-aliasing needs it.
    let step = TAU / f32(PROFILE_SAMPLES);
    let slope = (morph_radius(params, size, angle + step) - morph_radius(params, size, angle - step))
        / (2.0 * step * distance_to_center);
    let distance = (distance_to_center - edge) / sqrt(1.0 + slope * slope);
    return clamp(0.5 - distance, 0.0, 1.0);
}

// How much of the pixel at `local` (relative to the center) a rectangle of half-size
// `half_size` covers, its corners rounded by `radii` (clockwise from top-left).
fn rounded_coverage(local: vec2<f32>, half_size: vec2<f32>, radii: vec4<f32>) -> f32 {
    // The radius of the corner in this fragment's quadrant (y grows downward).
    let left = local.x < 0.0;
    let top = local.y < 0.0;
    let corner = select(
        select(radii.z, radii.y, top), // right: bottom-right, top-right
        select(radii.w, radii.x, top), // left: bottom-left, top-left
        left,
    );
    let radius = min(corner, min(half_size.x, half_size.y));
    // Signed distance to the rounded rectangle's edge (negative inside).
    let q = abs(local) - half_size + vec2<f32>(radius);
    let distance = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    return clamp(0.5 - distance, 0.0, 1.0);
}

// A sine wave stroked across the rect, about its vertical middle, with round ends.
// `params`: amplitude, wavelength, phase (radians), stroke width.
fn wave_coverage(local: vec2<f32>, half_size: vec2<f32>, params: vec4<f32>) -> f32 {
    let half_stroke = params.w * 0.5;
    let reach = max(half_size.x - half_stroke, 0.0);
    let x = clamp(local.x, -reach, reach);
    let k = TAU / max(params.y, 1e-3);
    let angle = (x + half_size.x) * k + params.z;
    let y = params.x * sin(angle);
    var distance: f32;
    if local.x == x {
        // Vertical distance, corrected by the slope to be the distance to the curve.
        let slope = params.x * k * cos(angle);
        distance = abs(local.y - y) / sqrt(1.0 + slope * slope);
    } else {
        distance = length(local - vec2<f32>(x, y));
    }
    return clamp(0.5 + half_stroke - distance, 0.0, 1.0);
}

// Glyph coverage blended in linear light makes light-on-dark text look bold and
// dark-on-light text look thin. Reshaping coverage reproduces sRGB-space blending
// exactly for white-on-black and black-on-white, interpolating by text brightness.
fn text_coverage(coverage: f32, color: vec4<f32>) -> f32 {
    let rgb = color.rgb / max(color.a, 1e-6);
    let brightness = sqrt(dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722)));
    let light = pow(coverage, TEXT_GAMMA);
    let dark = 1.0 - pow(1.0 - coverage, TEXT_GAMMA);
    return clamp(mix(dark, light, brightness), 0.0, 1.0);
}
