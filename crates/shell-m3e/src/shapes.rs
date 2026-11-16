//! Material 3 Expressive's shape library.
//!
//! The 35 shapes are ported from androidx's `MaterialShapes` and the `graphics-shapes`
//! library it builds on: each is a polygon whose corners are rounded by circular arcs
//! (optionally flanked by smoothing curves), normalized to fit a unit square.
//!
//! For drawing, each shape is an [`Outline`] for `shell-render`, which reduces it to a
//! *profile*: its distance from the center of that square, sampled at
//! [`PROFILE_SAMPLES`] evenly spaced angles. The shader rebuilds the outline from it,
//! and morphs between two shapes by blending their profiles. Every Material shape is
//! star-shaped around its center (each ray from it crosses the outline once), which is
//! what makes this exact enough; Material's own `Morph` instead matches corners between
//! shapes, so midway frames differ slightly.

use std::f32::consts::{PI, TAU};

use shell_render::{Outline, OutlineSource, PROFILE_SAMPLES};
use shell_ui::Shape;

/// A shape from Material 3 Expressive's library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialShape {
    Circle,
    Square,
    Slanted,
    Arch,
    Fan,
    Arrow,
    SemiCircle,
    Oval,
    Pill,
    Triangle,
    Diamond,
    ClamShell,
    Pentagon,
    Gem,
    VerySunny,
    Sunny,
    Cookie4Sided,
    Cookie6Sided,
    Cookie7Sided,
    Cookie9Sided,
    Cookie12Sided,
    Ghostish,
    Clover4Leaf,
    Clover8Leaf,
    Burst,
    SoftBurst,
    Boom,
    SoftBoom,
    Flower,
    Puffy,
    PuffyDiamond,
    PixelCircle,
    PixelTriangle,
    Bun,
    Heart,
}

impl MaterialShape {
    pub const ALL: [Self; 35] = [
        Self::Circle,
        Self::Square,
        Self::Slanted,
        Self::Arch,
        Self::Fan,
        Self::Arrow,
        Self::SemiCircle,
        Self::Oval,
        Self::Pill,
        Self::Triangle,
        Self::Diamond,
        Self::ClamShell,
        Self::Pentagon,
        Self::Gem,
        Self::VerySunny,
        Self::Sunny,
        Self::Cookie4Sided,
        Self::Cookie6Sided,
        Self::Cookie7Sided,
        Self::Cookie9Sided,
        Self::Cookie12Sided,
        Self::Ghostish,
        Self::Clover4Leaf,
        Self::Clover8Leaf,
        Self::Burst,
        Self::SoftBurst,
        Self::Boom,
        Self::SoftBoom,
        Self::Flower,
        Self::Puffy,
        Self::PuffyDiamond,
        Self::PixelCircle,
        Self::PixelTriangle,
        Self::Bun,
        Self::Heart,
    ];

    /// The outline, normalized into the unit square, as a closed polyline.
    pub fn polyline(self) -> Vec<[f32; 2]> {
        normalized(self.build())
    }

    /// The shape, for drawing.
    pub fn outline(self) -> Outline {
        // A static, so each shape's profile is computed and uploaded only once.
        static SHAPES: [MaterialShape; 35] = MaterialShape::ALL;
        Outline::new(&SHAPES[self as usize])
    }
}

impl OutlineSource for MaterialShape {
    /// Distance from (0.5, 0.5) to the outline at each of [`PROFILE_SAMPLES`] angles,
    /// starting at +x and turning toward +y (clockwise on screen).
    fn profile(&self) -> Vec<f32> {
        let outline = self.polyline();
        let center = [0.5, 0.5];
        let mut previous = 0.5;
        (0..PROFILE_SAMPLES)
            .map(|i| {
                let angle = i as f32 / PROFILE_SAMPLES as f32 * TAU;
                let distance = farthest_crossing(&outline, center, angle).unwrap_or(previous);
                previous = distance;
                distance
            })
            .collect()
    }
}

impl From<MaterialShape> for Outline {
    fn from(shape: MaterialShape) -> Self {
        shape.outline()
    }
}

impl From<MaterialShape> for Shape {
    fn from(shape: MaterialShape) -> Self {
        Shape::new(shape)
    }
}

impl MaterialShape {
    /// The `MaterialShapes` builders, before normalization.
    fn build(self) -> Vec<[f32; 2]> {
        use MaterialShape::*;
        let p = |x: f32, y: f32, r: Rounding| Point { at: [x, y], rounding: r };
        let sharp = Rounding::NONE;
        let r = |radius: f32| Rounding { radius, smoothing: 0.0 };
        let rs = |radius: f32, smoothing: f32| Rounding { radius, smoothing };

        match self {
            Circle => circle(10),
            Square => rounded(rectangle(1.0, 1.0, [r(0.3); 4])),
            Slanted => custom(
                &[p(0.926, 0.970, rs(0.189, 0.811)), p(-0.021, 0.967, rs(0.187, 0.057))],
                2,
                false,
            ),
            Arch => rotate(rounded(regular(4, &[r(1.0), r(1.0), r(0.2), r(0.2)])), -135.0),
            Fan => custom(
                &[
                    p(1.004, 1.000, rs(0.148, 0.417)),
                    p(0.000, 1.000, r(0.151)),
                    p(0.000, -0.003, r(0.148)),
                    p(0.978, 0.020, r(0.803)),
                ],
                1,
                false,
            ),
            Arrow => custom(
                &[
                    p(0.500, 0.892, r(0.313)),
                    p(-0.216, 1.050, r(0.207)),
                    p(0.499, -0.160, rs(0.215, 1.000)),
                    p(1.225, 1.060, r(0.211)),
                ],
                1,
                false,
            ),
            SemiCircle => rounded(rectangle(1.6, 1.0, [r(0.2), r(0.2), r(1.0), r(1.0)])),
            Oval => rotate(scale(circle(8), 1.0, 0.64), -45.0),
            Pill => custom(
                &[p(0.961, 0.039, r(0.426)), p(1.001, 0.428, sharp), p(1.000, 0.609, r(1.0))],
                2,
                true,
            ),
            Triangle => rotate(rounded(regular(3, &[r(0.2)])), -90.0),
            Diamond => {
                custom(&[p(0.500, 1.096, rs(0.151, 0.524)), p(0.040, 0.500, r(0.159))], 2, false)
            }
            ClamShell => custom(
                &[p(0.171, 0.841, r(0.159)), p(-0.020, 0.500, r(0.140)), p(0.170, 0.159, r(0.159))],
                2,
                false,
            ),
            Pentagon => custom(
                &[p(0.500, -0.009, r(0.172)), p(1.030, 0.365, r(0.164)), p(0.828, 0.970, r(0.169))],
                1,
                true,
            ),
            Gem => custom(
                &[
                    p(0.499, 1.023, rs(0.241, 0.778)),
                    p(-0.005, 0.792, r(0.208)),
                    p(0.073, 0.258, r(0.228)),
                    p(0.433, -0.000, r(0.491)),
                ],
                1,
                true,
            ),
            VerySunny => custom(&[p(0.500, 1.080, r(0.085)), p(0.358, 0.843, r(0.085))], 8, false),
            Sunny => rounded(star(8, 0.8, r(0.15))),
            Cookie4Sided => {
                custom(&[p(1.237, 1.236, r(0.258)), p(0.500, 0.918, r(0.233))], 4, false)
            }
            Cookie6Sided => {
                custom(&[p(0.723, 0.884, r(0.394)), p(0.500, 1.099, r(0.398))], 6, false)
            }
            Cookie7Sided => rotate(rounded(star(7, 0.75, r(0.5))), -90.0),
            Cookie9Sided => rotate(rounded(star(9, 0.8, r(0.5))), -90.0),
            Cookie12Sided => rotate(rounded(star(12, 0.8, r(0.5))), -90.0),
            Ghostish => custom(
                &[
                    p(0.500, 0.0, r(1.0)),
                    p(1.0, 0.0, r(1.0)),
                    p(1.0, 1.140, rs(0.254, 0.106)),
                    p(0.575, 0.906, r(0.253)),
                ],
                1,
                true,
            ),
            Clover4Leaf => custom(&[p(0.500, 0.074, sharp), p(0.725, -0.099, r(0.476))], 4, true),
            Clover8Leaf => custom(&[p(0.500, 0.036, sharp), p(0.758, -0.101, r(0.209))], 8, false),
            Burst => custom(&[p(0.500, -0.006, r(0.006)), p(0.592, 0.158, r(0.006))], 12, false),
            SoftBurst => custom(&[p(0.193, 0.277, r(0.053)), p(0.176, 0.055, r(0.053))], 10, false),
            Boom => custom(&[p(0.457, 0.296, r(0.007)), p(0.500, -0.051, r(0.007))], 15, false),
            SoftBoom => custom(
                &[
                    p(0.733, 0.454, sharp),
                    p(0.839, 0.437, r(0.532)),
                    p(0.949, 0.449, rs(0.439, 1.000)),
                    p(0.998, 0.478, r(0.174)),
                ],
                16,
                true,
            ),
            Flower => custom(
                &[p(0.370, 0.187, sharp), p(0.416, 0.049, r(0.381)), p(0.479, 0.001, r(0.095))],
                8,
                true,
            ),
            Puffy => scale(
                custom(
                    &[
                        p(0.500, 0.053, sharp),
                        p(0.545, -0.040, r(0.405)),
                        p(0.670, -0.035, r(0.426)),
                        p(0.717, 0.066, r(0.574)),
                        p(0.722, 0.128, sharp),
                        p(0.777, 0.002, r(0.360)),
                        p(0.914, 0.149, r(0.660)),
                        p(0.926, 0.289, r(0.660)),
                        p(0.881, 0.346, sharp),
                        p(0.940, 0.344, r(0.126)),
                        p(1.003, 0.437, r(0.255)),
                    ],
                    2,
                    true,
                ),
                1.0,
                0.742,
            ),
            PuffyDiamond => custom(
                &[p(0.870, 0.130, r(0.146)), p(0.818, 0.357, sharp), p(1.000, 0.332, r(0.853))],
                4,
                true,
            ),
            PixelCircle => custom(
                &[
                    p(0.500, 0.000, sharp),
                    p(0.704, 0.000, sharp),
                    p(0.704, 0.065, sharp),
                    p(0.843, 0.065, sharp),
                    p(0.843, 0.148, sharp),
                    p(0.926, 0.148, sharp),
                    p(0.926, 0.296, sharp),
                    p(1.000, 0.296, sharp),
                ],
                2,
                true,
            ),
            PixelTriangle => custom(
                &[
                    p(0.110, 0.500, sharp),
                    p(0.113, 0.000, sharp),
                    p(0.287, 0.000, sharp),
                    p(0.287, 0.087, sharp),
                    p(0.421, 0.087, sharp),
                    p(0.421, 0.170, sharp),
                    p(0.560, 0.170, sharp),
                    p(0.560, 0.265, sharp),
                    p(0.674, 0.265, sharp),
                    p(0.675, 0.344, sharp),
                    p(0.789, 0.344, sharp),
                    p(0.789, 0.439, sharp),
                    p(0.888, 0.439, sharp),
                ],
                1,
                true,
            ),
            Bun => custom(
                &[
                    p(0.796, 0.500, sharp),
                    p(0.853, 0.518, r(1.0)),
                    p(0.992, 0.631, r(1.0)),
                    p(0.968, 1.000, r(1.0)),
                ],
                2,
                true,
            ),
            Heart => custom(
                &[
                    p(0.500, 0.268, r(0.016)),
                    p(0.792, -0.066, r(0.958)),
                    p(1.064, 0.276, r(1.0)),
                    p(0.501, 0.946, r(0.129)),
                ],
                1,
                true,
            ),
        }
    }
}

// --- Polygon construction (graphics-shapes' builders and MaterialShapes' helpers) -------

/// `CornerRounding`: the radius of the arc that rounds a corner, and how far (0–1) the
/// rounding extends along the edges with smoothing curves either side of that arc.
#[derive(Clone, Copy, Debug)]
struct Rounding {
    radius: f32,
    smoothing: f32,
}

impl Rounding {
    const NONE: Self = Self { radius: 0.0, smoothing: 0.0 };
}

#[derive(Clone, Copy, Debug)]
struct Point {
    at: [f32; 2],
    rounding: Rounding,
}

/// `RoundedPolygon(numVertices, perVertexRounding)`: vertices on the unit circle from
/// +x; `roundings` repeats if shorter than the vertex count.
fn regular(count: usize, roundings: &[Rounding]) -> Vec<Point> {
    (0..count)
        .map(|i| Point {
            at: polar(1.0, TAU * i as f32 / count as f32),
            rounding: roundings[i % roundings.len()],
        })
        .collect()
}

/// `RoundedPolygon.circle`: a regular polygon whose corners round into its incircle.
fn circle(count: usize) -> Vec<[f32; 2]> {
    let radius = 1.0 / (PI / count as f32).cos();
    let mut points = regular(count, &[Rounding { radius: 1.0, smoothing: 0.0 }]);
    for point in &mut points {
        point.at = point.at.map(|c| c * radius);
    }
    rounded(points)
}

/// `RoundedPolygon.rectangle`, corners in its order: bottom-right, bottom-left,
/// top-left, top-right.
fn rectangle(width: f32, height: f32, roundings: [Rounding; 4]) -> Vec<Point> {
    let (x, y) = (width / 2.0, height / 2.0);
    [[x, y], [-x, y], [-x, -y], [x, -y]]
        .into_iter()
        .zip(roundings)
        .map(|(at, rounding)| Point { at, rounding })
        .collect()
}

/// `RoundedPolygon.star`: outer and inner vertices alternating, the first outer one on +x.
fn star(points: usize, inner_radius: f32, rounding: Rounding) -> Vec<Point> {
    (0..points * 2)
        .map(|i| Point {
            at: polar(if i % 2 == 0 { 1.0 } else { inner_radius }, PI * i as f32 / points as f32),
            rounding,
        })
        .collect()
}

/// `customPolygon`: `points` repeated `reps` times around (0.5, 0.5), or mirrored back
/// and forth across `2 * reps` sections.
fn custom(points: &[Point], reps: usize, mirroring: bool) -> Vec<[f32; 2]> {
    let center = [0.5, 0.5];
    let mut out = Vec::new();
    if mirroring {
        let offsets: Vec<[f32; 2]> = points.iter().map(|p| sub(p.at, center)).collect();
        let angles: Vec<f32> = offsets.iter().map(|o| o[1].atan2(o[0]).to_degrees()).collect();
        let distances: Vec<f32> = offsets.iter().map(|&o| length(o)).collect();
        let sections = reps * 2;
        let section = 360.0 / sections as f32;
        for it in 0..sections {
            for index in 0..points.len() {
                let i = if it % 2 == 0 { index } else { points.len() - 1 - index };
                if i > 0 || it % 2 == 0 {
                    let angle = section * it as f32
                        + if it % 2 == 0 {
                            angles[i]
                        } else {
                            section - angles[i] + 2.0 * angles[0]
                        };
                    let at = add(polar(distances[i], angle.to_radians()), center);
                    out.push(Point { at, rounding: points[i].rounding });
                }
            }
        }
    } else {
        for rep in 0..reps {
            let angle = (rep as f32 * 360.0 / reps as f32).to_radians();
            for point in points {
                out.push(Point {
                    at: rotate_point(point.at, angle, center),
                    rounding: point.rounding,
                });
            }
        }
    }
    rounded(out)
}

fn rotate(outline: Vec<[f32; 2]>, degrees: f32) -> Vec<[f32; 2]> {
    let angle = degrees.to_radians();
    outline.into_iter().map(|p| rotate_point(p, angle, [0.0, 0.0])).collect()
}

fn scale(outline: Vec<[f32; 2]>, sx: f32, sy: f32) -> Vec<[f32; 2]> {
    outline.into_iter().map(|[x, y]| [x * sx, y * sy]).collect()
}

/// `normalized()`: scaled uniformly to fit the unit square, centered in it.
fn normalized(outline: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in &outline {
        for axis in 0..2 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    let size = [max[0] - min[0], max[1] - min[1]];
    let side = size[0].max(size[1]);
    let offset = [(side - size[0]) / 2.0 - min[0], (side - size[1]) / 2.0 - min[1]];
    outline.into_iter().map(|[x, y]| [(x + offset[0]) / side, (y + offset[1]) / side]).collect()
}

// --- Corner rounding (graphics-shapes' RoundedPolygon / RoundedCorner) -------------------

struct Corner {
    vertex: [f32; 2],
    /// Unit vectors toward the previous and next vertices.
    to_prev: [f32; 2],
    to_next: [f32; 2],
    radius: f32,
    smoothing: f32,
    /// Distance along each edge to where the rounding arc touches it.
    round_cut: f32,
}

impl Corner {
    fn new(prev: [f32; 2], vertex: [f32; 2], next: [f32; 2], rounding: Rounding) -> Self {
        let (a, b) = (sub(prev, vertex), sub(next, vertex));
        let (la, lb) = (length(a), length(b));
        let mut corner = Self {
            vertex,
            to_prev: [0.0; 2],
            to_next: [0.0; 2],
            radius: 0.0,
            smoothing: 0.0,
            round_cut: 0.0,
        };
        if la > 0.0 && lb > 0.0 {
            corner.to_prev = scaled(a, 1.0 / la);
            corner.to_next = scaled(b, 1.0 / lb);
            corner.radius = rounding.radius;
            corner.smoothing = rounding.smoothing;
            let cos = dot(corner.to_prev, corner.to_next);
            let sin = (1.0 - cos * cos).max(0.0).sqrt();
            // tan(A/2) = sin A / (1 + cos A) = radius / cut
            corner.round_cut = if sin > 1e-3 { rounding.radius * (cos + 1.0) / sin } else { 0.0 };
        }
        corner
    }

    /// The cut including smoothing curves.
    fn cut(&self) -> f32 {
        (1.0 + self.smoothing) * self.round_cut
    }

    fn smoothing_within(&self, allowed: f32) -> f32 {
        if allowed > self.cut() {
            self.smoothing
        } else if allowed > self.round_cut {
            self.smoothing * (allowed - self.round_cut) / (self.cut() - self.round_cut)
        } else {
            0.0
        }
    }

    /// Appends the rounded corner, given the cuts its two edges can spare.
    fn trace(&self, allowed_prev: f32, allowed_next: f32, out: &mut Vec<[f32; 2]>) {
        let allowed = allowed_prev.min(allowed_next);
        if self.round_cut < 1e-4 || allowed < 1e-4 || self.radius < 1e-4 {
            out.push(self.vertex);
            return;
        }
        let round_cut = allowed.min(self.round_cut);
        let radius = self.radius * round_cut / self.round_cut;
        let center_distance = (radius * radius + round_cut * round_cut).sqrt();
        let bisector = normalize(add(self.to_prev, self.to_next));
        let center = add(self.vertex, scaled(bisector, center_distance));
        let touch_prev = add(self.vertex, scaled(self.to_prev, round_cut));
        let touch_next = add(self.vertex, scaled(self.to_next, round_cut));

        let flank_in = self.flank(
            round_cut,
            self.smoothing_within(allowed_prev),
            self.to_prev,
            touch_prev,
            touch_next,
            center,
            radius,
        );
        let flank_out = self.flank(
            round_cut,
            self.smoothing_within(allowed_next),
            self.to_next,
            touch_next,
            touch_prev,
            center,
            radius,
        );

        sample_cubic(flank_in, out);
        sample_arc(center, flank_in[3], flank_out[3], out);
        // The outgoing flank runs from the arc back out to the edge.
        sample_cubic([flank_out[3], flank_out[2], flank_out[1], flank_out[0]], out);
    }

    /// `computeFlankingCurve`: a cubic from the edge (`side` direction) onto the arc.
    #[allow(clippy::too_many_arguments)]
    fn flank(
        &self,
        round_cut: f32,
        smoothing: f32,
        side: [f32; 2],
        touch: [f32; 2],
        other_touch: [f32; 2],
        center: [f32; 2],
        radius: f32,
    ) -> [[f32; 2]; 4] {
        let start = add(self.vertex, scaled(side, round_cut * (1.0 + smoothing)));
        let toward = lerp(touch, scaled(add(touch, other_touch), 0.5), smoothing);
        let end = add(center, scaled(normalize(sub(toward, center)), radius));
        let tangent = perpendicular(sub(end, center));
        let anchor_end = line_intersection(start, side, end, tangent).unwrap_or(touch);
        let anchor_start = scaled(add(start, scaled(anchor_end, 2.0)), 1.0 / 3.0);
        [start, anchor_start, anchor_end, end]
    }
}

/// Rounds a polygon's corners, sharing each edge between the cuts of its two corners
/// (rounding first, then smoothing) when it is too short for both.
fn rounded(points: Vec<Point>) -> Vec<[f32; 2]> {
    let n = points.len();
    let corners: Vec<Corner> = (0..n)
        .map(|i| {
            Corner::new(
                points[(i + n - 1) % n].at,
                points[i].at,
                points[(i + 1) % n].at,
                points[i].rounding,
            )
        })
        .collect();
    // For edge i (from vertex i to i + 1): the fraction of each corner's round cut and of
    // its smoothing cut that fit.
    let adjust: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            let (a, b) = (&corners[i], &corners[(i + 1) % n]);
            let round = a.round_cut + b.round_cut;
            let full = a.cut() + b.cut();
            let side = length(sub(points[(i + 1) % n].at, points[i].at));
            if round > side {
                (side / round, 0.0)
            } else if full > side {
                (1.0, (side - round) / (full - round))
            } else {
                (1.0, 1.0)
            }
        })
        .collect();

    let mut out = Vec::new();
    for (i, corner) in corners.iter().enumerate() {
        let allowed = |(round, smooth): (f32, f32)| {
            corner.round_cut * round + (corner.cut() - corner.round_cut) * smooth
        };
        corner.trace(allowed(adjust[(i + n - 1) % n]), allowed(adjust[i]), &mut out);
    }
    out
}

fn sample_cubic([p0, p1, p2, p3]: [[f32; 2]; 4], out: &mut Vec<[f32; 2]>) {
    const STEPS: usize = 12;
    for step in 0..=STEPS {
        let t = step as f32 / STEPS as f32;
        let u = 1.0 - t;
        let point = add(
            add(scaled(p0, u * u * u), scaled(p1, 3.0 * u * u * t)),
            add(scaled(p2, 3.0 * u * t * t), scaled(p3, t * t * t)),
        );
        out.push(point);
    }
}

/// The shorter arc around `center` from `from` to `to` (corner arcs are under 180°).
fn sample_arc(center: [f32; 2], from: [f32; 2], to: [f32; 2], out: &mut Vec<[f32; 2]>) {
    let radius = length(sub(from, center));
    let start = (from[1] - center[1]).atan2(from[0] - center[0]);
    let end = (to[1] - center[1]).atan2(to[0] - center[0]);
    let mut sweep = end - start;
    if sweep > PI {
        sweep -= TAU;
    } else if sweep < -PI {
        sweep += TAU;
    }
    let steps = ((sweep.abs() / (PI / 32.0)).ceil() as usize).max(1);
    for step in 0..=steps {
        let angle = start + sweep * step as f32 / steps as f32;
        out.push(add(center, polar(radius, angle)));
    }
}

/// Distance from `center` to the farthest point where a ray at `angle` crosses the
/// closed `outline`.
fn farthest_crossing(outline: &[[f32; 2]], center: [f32; 2], angle: f32) -> Option<f32> {
    let direction = polar(1.0, angle);
    let mut farthest: Option<f32> = None;
    for i in 0..outline.len() {
        let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
        let edge = sub(b, a);
        let denominator = cross(direction, edge);
        if denominator.abs() < 1e-9 {
            continue;
        }
        let to_a = sub(a, center);
        let t = cross(to_a, edge) / denominator;
        let u = cross(to_a, direction) / denominator;
        if t > 0.0 && (0.0..=1.0).contains(&u) {
            farthest = Some(farthest.map_or(t, |f| f.max(t)));
        }
    }
    farthest
}

// --- 2D vector helpers ------------------------------------------------------------------

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn scaled(a: [f32; 2], s: f32) -> [f32; 2] {
    [a[0] * s, a[1] * s]
}
fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}
fn length(a: [f32; 2]) -> f32 {
    dot(a, a).sqrt()
}
fn normalize(a: [f32; 2]) -> [f32; 2] {
    let l = length(a);
    if l > 0.0 { scaled(a, 1.0 / l) } else { a }
}
fn perpendicular([x, y]: [f32; 2]) -> [f32; 2] {
    [-y, x]
}
fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    add(a, scaled(sub(b, a), t))
}
fn polar(radius: f32, angle: f32) -> [f32; 2] {
    [radius * angle.cos(), radius * angle.sin()]
}
fn rotate_point(p: [f32; 2], angle: f32, center: [f32; 2]) -> [f32; 2] {
    let [x, y] = sub(p, center);
    let (sin, cos) = angle.sin_cos();
    add([x * cos - y * sin, x * sin + y * cos], center)
}
/// Where the lines through `p0` along `d0` and through `p1` along `d1` meet.
fn line_intersection(p0: [f32; 2], d0: [f32; 2], p1: [f32; 2], d1: [f32; 2]) -> Option<[f32; 2]> {
    let denominator = cross(d0, d1);
    if denominator.abs() < 1e-6 {
        return None;
    }
    let t = cross(sub(p1, p0), d1) / denominator;
    Some(add(p0, scaled(d0, t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_fills_the_unit_square() {
        for shape in MaterialShape::ALL {
            let outline = shape.polyline();
            let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
            for p in &outline {
                assert!(p[0].is_finite() && p[1].is_finite(), "{shape:?}");
                for axis in 0..2 {
                    min[axis] = min[axis].min(p[axis]);
                    max[axis] = max[axis].max(p[axis]);
                }
            }
            let side = (max[0] - min[0]).max(max[1] - min[1]);
            assert!((side - 1.0).abs() < 1e-3, "{shape:?}: side {side}");
            assert!(min.iter().all(|&m| m > -1e-3) && max.iter().all(|&m| m < 1.0 + 1e-3));

            let profile = shape.profile();
            assert!(profile.iter().all(|&r| r > 0.05 && r < 0.75), "{shape:?}: {profile:?}");
        }
    }

    #[test]
    fn circle_is_round_and_square_is_rounded() {
        for r in MaterialShape::Circle.profile() {
            assert!((r - 0.5).abs() < 0.002, "circle radius {r}");
        }
        let square = MaterialShape::Square.profile();
        // Edge midpoint (angle 0) is on the side; the diagonal is cut by the 0.3 rounding:
        // the corner arc's center sits 0.3 in from each side, so the diagonal reaches
        // 0.2·√2 + 0.3 = 0.583 instead of the sharp corner's 0.707.
        assert!((square[0] - 0.5).abs() < 1e-3, "side {}", square[0]);
        let diagonal = square[PROFILE_SAMPLES / 8];
        assert!((diagonal - 0.583).abs() < 0.005, "diagonal {diagonal}");
    }

    #[test]
    fn cookie9_has_nine_points_with_one_facing_up() {
        let profile = MaterialShape::Cookie9Sided.profile();
        let n = profile.len();
        let peaks = (0..n)
            .filter(|&i| {
                profile[i] > profile[(i + n - 1) % n] && profile[i] >= profile[(i + 1) % n]
            })
            .count();
        assert_eq!(peaks, 9);
        // Up is -90°, three quarters of the way around. With an odd number of points,
        // centering the bounding box (as Material does) moves the center off the
        // star's, so the top point is a peak but not the farthest one.
        let up = n * 3 / 4;
        let window = n / 36;
        assert!(
            (up - window..=up + window).all(|i| profile[i] <= profile[up] + 1e-4),
            "a point faces up"
        );
    }
}
