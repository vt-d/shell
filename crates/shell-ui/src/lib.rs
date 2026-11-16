//! Declarative UI on top of Wayland, taffy and [`shell_render`].
//!
//! - A [`Window`] says *where* a surface lives: its role (currently a layer-shell layer),
//!   anchors, size along each axis ([`Extent`]), margins and exclusive zone.
//! - An [`Element`] tree says *what* it shows: flexbox boxes laid out by taffy, painted as
//!   rounded quads or [`Shape`]s, and text shaped by cosmic-text and measured as part of
//!   layout.
//! - [`Ui`] runs it all as the data of a calloop event loop, redrawing a window only when
//!   it changed and pacing frames with the compositor's frame callbacks.
//! - [`motion`] springs animate anything a view computes; a window animates frame by
//!   frame only while one is moving.
//! - Values [provided](Ui::provide) to the UI, such as a design system's theme, reach every
//!   view through its [`Frame`], and rebuild the windows when they change.
//!
//! The UI imposes no design system; `shell-m3e` adds Material 3 Expressive on top.
//!
//! ```ignore
//! let mut event_loop = calloop::EventLoop::try_new()?;
//! let mut ui = Ui::connect(&event_loop)?;
//! ui.open(
//!     Window::layer_shell("bar")
//!         .anchor(Anchor::TOP)
//!         .width(Extent::Fill)
//!         .height(Extent::Fixed(32))
//!         .exclusive()
//!         .root(Element::row()),
//! );
//! event_loop.run(None, &mut ui, |_| {})?;
//! ```

mod app;
mod element;
mod env;
mod layout;
mod window;

pub use app::{Key, KeyEvent, PointerEvent, PointerKind, Ui, WindowId};
pub use element::{Element, FontFamily, Image, Shape, Symbol, Wave};
pub use window::{ExclusiveZone, Extent, Frame, LayerOptions, Margin, Role, View, Window};

pub use shell_motion as motion;
pub use shell_render::{
    Axis, Color, ImageFile, MAX_AXES, Outline, OutlineSource, PROFILE_SAMPLES, SymbolFont,
};
pub use smithay_client_toolkit::reexports::calloop;
pub use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
pub use taffy::{AlignItems, FlexDirection, JustifyContent};
