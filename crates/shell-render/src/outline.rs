//! Shapes the renderer fills, described in polar form.

use std::{
    any::{Any, TypeId},
    fmt, ptr,
};

/// How many radii describe an outline.
pub const PROFILE_SAMPLES: usize = 256;

/// The source of an [`Outline`]: a closed shape in the unit square that every ray from
/// the square's center (0.5, 0.5) crosses once, so that its distance from the center at
/// each angle describes it.
pub trait OutlineSource: Any + Sync {
    /// The outline's distance from (0.5, 0.5), as a fraction of the square's side, at
    /// [`PROFILE_SAMPLES`] evenly spaced angles, clockwise on screen starting from +x.
    ///
    /// Called once per source, the first time it is drawn.
    fn profile(&self) -> Vec<f32>;
}

/// A shape to fill: a handle on a `'static` [`OutlineSource`].
///
/// Two outlines are the same when they refer to the same source value. The renderer
/// computes and uploads each one's profile once, so sources should be statics (or
/// constants) rather than built per frame.
#[derive(Clone, Copy)]
pub struct Outline(&'static dyn OutlineSource);

impl Outline {
    pub const fn new(source: &'static dyn OutlineSource) -> Self {
        Self(source)
    }

    /// A circle filling the square.
    pub const CIRCLE: Self = Self(&Circle);

    pub(crate) fn profile(self) -> Vec<f32> {
        self.0.profile()
    }

    /// Identifies the source: its address, and its type, which tells apart zero-sized
    /// sources that share an address.
    pub(crate) fn key(self) -> (TypeId, usize) {
        let any: &dyn Any = self.0;
        (any.type_id(), ptr::from_ref(self.0).cast::<()>() as usize)
    }
}

impl PartialEq for Outline {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl fmt::Debug for Outline {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let any: &dyn Any = self.0;
        write!(f, "Outline({:?} at {:p})", any.type_id(), ptr::from_ref(self.0).cast::<()>())
    }
}

struct Circle;

impl OutlineSource for Circle {
    fn profile(&self) -> Vec<f32> {
        vec![0.5; PROFILE_SAMPLES]
    }
}
