//! Fonts: discovery, fallback and rasterization.
//!
//! Shaping and line layout are done by cosmic-text (HarfRust shaping, BiDi, line
//! breaking, per-script font fallback over the system's fonts via fontconfig). Glyphs are
//! rasterized by swash, hinted, at physical size and quarter-pixel horizontal offsets,
//! and cached in the renderer's [glyph atlas](super::atlas).

use std::sync::Arc;

use cosmic_text::{CacheKey, FontSystem, SwashCache, SwashImage, fontdb};

/// The system's fonts and the rasterizer that draws them.
pub struct Fonts {
    system: FontSystem,
    scaler: SwashCache,
}

impl Fonts {
    /// Loads the system font database. This scans the font directories, so do it once.
    pub fn new() -> Self {
        Self { system: FontSystem::new(), scaler: SwashCache::new() }
    }

    /// The font database and shaping state, for laying out [`cosmic_text::Buffer`]s.
    pub fn system(&mut self) -> &mut FontSystem {
        &mut self.system
    }

    /// Adds a font (a TrueType/OpenType file's contents) to those text can be set in.
    pub fn add(&mut self, data: &'static [u8]) {
        self.system.db_mut().load_font_source(fontdb::Source::Binary(Arc::new(data)));
    }

    /// Makes `family` what [generic sans-serif](cosmic_text::Family::SansSerif) text is
    /// set in, instead of fontconfig's choice.
    pub fn set_sans_serif(&mut self, family: &str) {
        self.system.db_mut().set_sans_serif_family(family);
    }

    /// Rasterizes one glyph. Results are cached by the atlas, not here.
    pub(super) fn rasterize(&mut self, key: CacheKey) -> Option<SwashImage> {
        self.scaler.get_image_uncached(&mut self.system, key)
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}
