//! Material 3 Expressive for `shell-ui`.
//!
//! - [`MaterialShape`]: Material's 35-shape library, usable anywhere `shell-ui` takes a
//!   [`Shape`](shell_ui::Shape) or an [`Outline`](shell_ui::Outline), and morphable into
//!   one another.
//! - [`Theme`]: a Material You color scheme read from [matugen](https://github.com/InioX/matugen)
//!   output, reloaded live by [`watch_theme`] and read in views with
//!   [`ThemeExt::theme`].
//! - [`Icon`]: Material Symbols, with fill, weight, grade and optical size.
//! - [`use_typeface`]: Google Sans Flex, Material 3 Expressive's typeface, as the default.
//! - [`motion`]: Material's spring specs.
//! - [`components`]: controls that take pointer input: buttons and sliders.

pub mod components;
pub mod icons;
pub mod motion;
mod shapes;
pub mod theme;
pub mod typeface;

pub use icons::Icon;
pub use shapes::MaterialShape;
pub use theme::{Theme, ThemeError, ThemeExt, ThemeMode, watch_theme};
pub use typeface::use_typeface;
