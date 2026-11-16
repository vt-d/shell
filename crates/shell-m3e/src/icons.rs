//! Material Symbols: the icons of Material 3 Expressive.
//!
//! Material Symbols is a variable font with four axes, all of which an [`Icon`] can set
//! and animate:
//!
//! - fill, 0–1: outlined to filled. Expressive UIs fill an icon to show it's active
//!   or selected, animating between the two.
//! - weight, 100–700: stroke weight, normally matched to the text beside it.
//! - grade, −25–200: finer emphasis. Material uses −25 for light icons on dark
//!   backgrounds, which otherwise look heavier than dark-on-light ones.
//! - optical size, 20–48: detail for the size drawn at; set from the icon's size.
//!
//! The Rounded style, the one Expressive uses, is built into the binary. Its path comes
//! from `MATERIAL_SYMBOLS_ROUNDED`, which `nix develop` and the flake's package set.
//!
//! Icons are named as on <https://fonts.google.com/icons>, such as `"wifi"` or
//! `"volume_up"`.

use shell_ui::{Axis, Symbol, SymbolFont};

static FONT: &[u8] = include_bytes!(env!(
    "MATERIAL_SYMBOLS_ROUNDED",
    "set MATERIAL_SYMBOLS_ROUNDED to the path of MaterialSymbolsRounded[FILL,GRAD,opsz,wght].ttf \
     (`nix develop` does)"
));

/// The Material Symbols Rounded font.
pub fn font() -> SymbolFont {
    SymbolFont::new(FONT)
}

/// A Material Symbol, for [`Element::symbol`](shell_ui::Element::symbol). It is drawn
/// in the text color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Icon {
    pub name: &'static str,
    /// The side of its square, in logical pixels. Defaults to Material's 24.
    pub size: f32,
    pub fill: f32,
    pub weight: f32,
    pub grade: f32,
}

impl Icon {
    pub const fn new(name: &'static str) -> Self {
        Self { name, size: 24.0, fill: 0.0, weight: 400.0, grade: 0.0 }
    }

    pub const fn size(mut self, px: f32) -> Self {
        self.size = px;
        self
    }

    /// 0 is outlined, 1 filled; animate between them.
    pub const fn fill(mut self, fill: f32) -> Self {
        self.fill = fill;
        self
    }

    pub const fn weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    pub const fn grade(mut self, grade: f32) -> Self {
        self.grade = grade;
        self
    }

    /// Where the icon actually draws within its square: `[left, top, right, bottom]` in
    /// logical pixels from the square's top-left corner. Material draws some symbols
    /// nearly to the square's edges and others well inside it, so icons spaced by their
    /// squares look unevenly spaced; space them by this instead. `None` if there's no
    /// such icon.
    pub fn ink_bounds(&self) -> Option<[f32; 4]> {
        let symbol = Symbol::from(*self);
        symbol.font.ink_bounds(symbol.name, symbol.size, &symbol.axes)
    }
}

impl From<Icon> for Symbol {
    fn from(icon: Icon) -> Self {
        Symbol {
            font: font(),
            name: icon.name,
            size: icon.size,
            axes: [
                Axis::new(*b"FILL", icon.fill.clamp(0.0, 1.0)),
                Axis::new(*b"wght", icon.weight.clamp(100.0, 700.0)),
                Axis::new(*b"GRAD", icon.grade.clamp(-25.0, 200.0)),
                Axis::new(*b"opsz", icon.size.clamp(20.0, 48.0)),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_to_symbols() {
        for name in ["wifi", "volume_up", "volume_off", "lan", "network_wifi_2_bar"] {
            assert!(font().contains(name), "{name}");
        }
        assert!(!font().contains("not_an_icon_name"));
    }
}
