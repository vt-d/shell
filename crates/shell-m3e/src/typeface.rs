//! Google Sans Flex: the typeface of Material 3 Expressive, as on Android 16 and Pixel.
//!
//! A variable font (weight 1–1000, width, optical size, slant and roundness), built into
//! the binary. Its path comes from `GOOGLE_SANS_FLEX`, which `nix develop` and the
//! flake's package set.

use shell_ui::Ui;

static FONT: &[u8] = include_bytes!(env!(
    "GOOGLE_SANS_FLEX",
    "set GOOGLE_SANS_FLEX to the path of GoogleSansFlex[ROND,opsz,slnt,wdth,wght].ttf \
     (`nix develop` does)"
));

/// Its family name, for [`shell_ui::FontFamily::Name`].
pub const FAMILY: &str = "Google Sans Flex";

/// Makes Google Sans Flex available, and what text is set in by default. Call it before
/// opening windows.
pub fn use_typeface(ui: &mut Ui) {
    ui.add_font(FONT);
    ui.set_default_font(FAMILY);
}
