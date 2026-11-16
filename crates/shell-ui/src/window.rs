//! Windows: a declarative [`Window`] description, and the live [`WindowState`] behind it.

use smithay_client_toolkit::shell::{
    WaylandSurface,
    wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface},
};
use wayland_client::{QueueHandle, protocol::wl_surface::WlSurface};

use std::time::{Duration, Instant};

use shell_motion::Spring;
use shell_render::{Color, RenderSurface, Renderer, Scene};

use crate::{
    Ui,
    element::Element,
    env::Env,
    layout::{Hit, Layout},
};

/// How a window's size is decided along one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extent {
    /// Span the whole output along this axis. Implies anchoring to both edges of it.
    Fill,
    /// A fixed size in logical pixels.
    Fixed(u32),
    /// Shrink-wrap the window's content, as measured by layout.
    Fit,
}

/// How a layer window reserves space from other windows. See `zwlr_layer_surface_v1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExclusiveZone {
    /// Reserve nothing, but move to avoid other windows' exclusive zones.
    #[default]
    None,
    /// Reserve the window's own size (plus margin) along the edge it is attached to.
    Auto,
    /// Reserve this many logical pixels from the attached edge.
    Fixed(u32),
    /// Ignore other windows' exclusive zones and extend to the anchored edges.
    Ignore,
}

/// Distance from each anchored edge, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Margin {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

impl Margin {
    pub const fn new(top: i32, right: i32, bottom: i32, left: i32) -> Self {
        Self { top, right, bottom, left }
    }

    pub const fn all(px: i32) -> Self {
        Self::new(px, px, px, px)
    }

    pub const fn symmetric(vertical: i32, horizontal: i32) -> Self {
        Self::new(vertical, horizontal, vertical, horizontal)
    }
}

/// Options for a window placed with the layer-shell protocol.
#[derive(Clone, Debug)]
pub struct LayerOptions {
    pub namespace: String,
    pub layer: Layer,
    pub anchor: Anchor,
    pub width: Extent,
    pub height: Extent,
    pub exclusive_zone: ExclusiveZone,
    pub margin: Margin,
    pub keyboard: KeyboardInteractivity,
}

impl LayerOptions {
    /// The anchor actually requested: the explicit anchor plus both edges of any
    /// [`Extent::Fill`] axis.
    fn effective_anchor(&self) -> Anchor {
        let mut anchor = self.anchor;
        if self.width == Extent::Fill {
            anchor |= Anchor::LEFT | Anchor::RIGHT;
        }
        if self.height == Extent::Fill {
            anchor |= Anchor::TOP | Anchor::BOTTOM;
        }
        anchor
    }

    /// The `set_exclusive_zone` value for a window of logical size `(width, height)`.
    fn exclusive_zone_for(&self, (width, height): (u32, u32)) -> i32 {
        match self.exclusive_zone {
            ExclusiveZone::None => 0,
            ExclusiveZone::Ignore => -1,
            ExclusiveZone::Fixed(px) => px as i32,
            ExclusiveZone::Auto => {
                // The zone extends from the single edge the window is attached to. The
                // protocol counts the margin as part of the zone.
                let a = self.effective_anchor();
                let m = self.margin;
                match (a.contains(Anchor::TOP), a.contains(Anchor::BOTTOM)) {
                    (true, false) if !corner(a) => return height as i32 + m.top,
                    (false, true) if !corner(a) => return height as i32 + m.bottom,
                    _ => {}
                }
                match (a.contains(Anchor::LEFT), a.contains(Anchor::RIGHT)) {
                    (true, false) if !corner(a) => width as i32 + m.left,
                    (false, true) if !corner(a) => width as i32 + m.right,
                    // Unanchored, cornered or stretched on both axes: a zone is meaningless.
                    _ => 0,
                }
            }
        }
    }
}

/// Anchored to exactly one horizontal and one vertical edge.
fn corner(a: Anchor) -> bool {
    let vertical = a.contains(Anchor::TOP) != a.contains(Anchor::BOTTOM);
    let horizontal = a.contains(Anchor::LEFT) != a.contains(Anchor::RIGHT);
    vertical && horizontal
}

/// What kind of surface a window is, and how the compositor should place it.
#[derive(Clone, Debug)]
pub enum Role {
    /// A shell layer (bar, dock, overlay, wallpaper...) via `zwlr_layer_shell_v1`.
    Layer(LayerOptions),
}

/// A declarative description of a window. Build one and pass it to [`Ui::open`].
///
/// ```ignore
/// Window::layer_shell("bar")
///     .anchor(Anchor::TOP)
///     .width(Extent::Fill)
///     .height(Extent::Fixed(32))
///     .exclusive()
///     .root(Element::row())
/// ```
#[derive(Clone, Debug)]
pub struct Window {
    pub(crate) role: Role,
    pub(crate) background: Color,
    pub(crate) root: Element,
}

impl Window {
    /// A layer-shell window. Defaults: [`Layer::Top`], unanchored (centered by the
    /// compositor), sized to fit its content, no exclusive zone, no keyboard focus.
    pub fn layer_shell(namespace: impl Into<String>) -> Self {
        Self {
            role: Role::Layer(LayerOptions {
                namespace: namespace.into(),
                layer: Layer::Top,
                anchor: Anchor::empty(),
                width: Extent::Fit,
                height: Extent::Fit,
                exclusive_zone: ExclusiveZone::None,
                margin: Margin::default(),
                keyboard: KeyboardInteractivity::None,
            }),
            background: Color::TRANSPARENT,
            root: Element::new(),
        }
    }

    fn layer_options(&mut self) -> &mut LayerOptions {
        match &mut self.role {
            Role::Layer(options) => options,
        }
    }

    /// Which layer of the shell stack to live in.
    pub fn layer(mut self, layer: Layer) -> Self {
        self.layer_options().layer = layer;
        self
    }

    /// Edges to attach to, e.g. `Anchor::TOP | Anchor::RIGHT` for a top-right corner.
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.layer_options().anchor = anchor;
        self
    }

    pub fn width(mut self, width: Extent) -> Self {
        self.layer_options().width = width;
        self
    }

    pub fn height(mut self, height: Extent) -> Self {
        self.layer_options().height = height;
        self
    }

    pub fn exclusive_zone(mut self, zone: ExclusiveZone) -> Self {
        self.layer_options().exclusive_zone = zone;
        self
    }

    /// Shorthand for [`ExclusiveZone::Auto`]: keep other windows clear of this one.
    pub fn exclusive(self) -> Self {
        self.exclusive_zone(ExclusiveZone::Auto)
    }

    pub fn margin(mut self, margin: Margin) -> Self {
        self.layer_options().margin = margin;
        self
    }

    pub fn keyboard(mut self, keyboard: KeyboardInteractivity) -> Self {
        self.layer_options().keyboard = keyboard;
        self
    }

    /// Fill color behind the content. Transparent by default.
    pub fn background(mut self, color: Color) -> Self {
        self.background = color;
        self
    }

    pub fn root(mut self, root: Element) -> Self {
        self.root = root;
        self
    }
}

/// What a view is built with: the values [provided](Ui::provide) to every view, and the
/// time of the frame being built, against which animations are advanced.
pub struct Frame<'a> {
    env: &'a Env,
    now: Instant,
    animating: bool,
    /// How long the view can wait to be built again, if it asked to be.
    again: Option<Duration>,
    close: bool,
}

impl<'a> Frame<'a> {
    fn new(env: &'a Env) -> Self {
        Self { env, now: Instant::now(), animating: false, again: None, close: false }
    }

    /// The value of type `T` last [provided](Ui::provide), if any.
    pub fn get<T: 'static>(&self) -> Option<&'a T> {
        self.env.get()
    }

    pub fn now(&self) -> Instant {
        self.now
    }

    /// Advances `spring` to this frame and returns its value. While it is still moving,
    /// the view is built again on the next frame, in step with the display.
    pub fn animate(&mut self, spring: &mut Spring) -> f32 {
        let value = spring.update(self.now);
        self.animating |= spring.is_moving();
        value
    }

    /// Builds the view again on the next frame, for animation not driven by a [`Spring`].
    pub fn request_frame(&mut self) {
        self.animating = true;
    }

    /// Builds the view again once `wait` has passed, rather than on the next frame: for
    /// slow, ambient motion, which looks the same drawn a few times less often than the
    /// display refreshes and costs that much less. If anything else is animating, the
    /// view is built again on the next frame as usual.
    pub fn request_frame_after(&mut self, wait: Duration) {
        self.again = Some(self.again.map_or(wait, |again| again.min(wait)));
    }

    /// When to build the view again, if not on the next frame.
    fn wake(&self) -> Option<Instant> {
        self.again.filter(|_| !self.animating).map(|wait| self.now + wait)
    }

    /// Closes the window once this view is built: for a window that has finished
    /// animating away. Its [`WindowId`](crate::WindowId) is no use afterwards.
    pub fn close(&mut self) {
        self.close = true;
    }
}

/// Builds a window's description. [`Ui`] builds it again whenever a [provided](Ui::provide)
/// value changes, the window is [refreshed](Ui::refresh), or an animation it runs needs
/// another frame, so read provided values (a theme, say) from the [`Frame`] and advance
/// animations with [`Frame::animate`].
///
/// Implemented by closures and functions taking `&mut Frame` and returning a [`Window`]
/// (closures need the argument's type written out: `|frame: &mut Frame| ...`), and by
/// [`Window`] itself, for content that never changes.
///
/// Only the background and content of a rebuilt window are applied. Its role (layer,
/// anchors, size, margins...) is fixed when the window is opened.
pub trait View: 'static {
    fn build(&mut self, frame: &mut Frame) -> Window;
}

impl<F: FnMut(&mut Frame) -> Window + 'static> View for F {
    fn build(&mut self, frame: &mut Frame) -> Window {
        self(frame)
    }
}

impl View for Window {
    fn build(&mut self, _: &mut Frame) -> Window {
        self.clone()
    }
}

/// A live window: its Wayland surface, swapchain, layout, and redraw scheduling.
///
/// Redraws are demand-driven and paced by the compositor:
///
/// - Anything that changes what's on screen marks the window dirty. Nothing is drawn
///   while it is clean.
/// - A view that is animating is built again on every frame callback, until its
///   animations settle.
/// - Each presented frame carries a `wl_surface.frame` callback. Until the compositor fires
///   it (at the output's refresh rate, and not at all while hidden), further redraws are
///   held back and coalesced into a single frame.
/// - Size requests (`set_size`) are double-buffered and ride along with the commit of the
///   next drawn frame; the compositor then answers with a configure.
pub(crate) struct WindowState {
    // Declared before `layer` so it drops first: the layer surface destroys the wl_surface.
    target: RenderSurface,
    layer: LayerSurface,
    qh: QueueHandle<Ui>,
    options: LayerOptions,
    view: Box<dyn View>,
    background: Color,
    layout: Layout,
    /// Logical size last sent with `set_size` (0 on an axis means "fill").
    requested: (u32, u32),
    /// Logical size from the latest configure.
    size: (u32, u32),
    scale: i32,
    configured: bool,
    dirty: bool,
    frame_pending: bool,
    /// The view is mid-animation and must be built again for the next frame.
    animating: bool,
    /// When the view, not animating but in slow motion, is to be built again; and whether
    /// a timer is set to do it.
    wake: Option<Instant>,
    wake_armed: bool,
    /// The view asked for the window to be closed.
    close_requested: bool,
    /// Where the pointer targets were when last drawn, back to front.
    hits: Vec<Hit>,
}

impl WindowState {
    pub fn new(
        mut view: Box<dyn View>,
        layer_shell: &LayerShell,
        qh: &QueueHandle<Ui>,
        wl_surface: WlSurface,
        target: RenderSurface,
        renderer: &mut Renderer,
        env: &Env,
    ) -> Self {
        let mut frame = Frame::new(env);
        let window = view.build(&mut frame);
        let Role::Layer(options) = window.role;
        let layer = layer_shell.create_layer_surface(
            qh,
            wl_surface,
            options.layer,
            Some(options.namespace.clone()),
            None,
        );
        layer.set_anchor(options.effective_anchor());
        let m = options.margin;
        layer.set_margin(m.top, m.right, m.bottom, m.left);
        layer.set_keyboard_interactivity(options.keyboard);

        let mut state = Self {
            target,
            layer,
            qh: qh.clone(),
            options,
            view,
            background: window.background,
            layout: Layout::new(&window.root),
            requested: (u32::MAX, u32::MAX),
            size: (0, 0),
            scale: 1,
            configured: false,
            dirty: true,
            frame_pending: false,
            animating: frame.animating,
            wake: frame.wake(),
            wake_armed: false,
            close_requested: frame.close,
            hits: Vec::new(),
        };
        state.update_requested_size(renderer);
        // Initial commit without a buffer; the compositor answers with a configure.
        state.layer.commit();
        state
    }

    pub fn wl_surface(&self) -> &WlSurface {
        self.layer.wl_surface()
    }

    /// Its size in logical pixels; zero until the compositor has said.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Whether the view has asked for the window to be closed.
    pub fn close_requested(&self) -> bool {
        self.close_requested
    }

    /// The topmost pointer target at `x`, `y` (logical pixels), as last drawn.
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits.iter().rev().find(|hit| hit.contains(x, y)).copied()
    }

    /// Builds the view again and takes on its background and content.
    pub fn rebuild(&mut self, renderer: &mut Renderer, env: &Env) {
        let mut frame = Frame::new(env);
        let window = self.view.build(&mut frame);
        self.animating = frame.animating;
        self.wake = frame.wake();
        self.close_requested |= frame.close;
        self.background = window.background;
        self.layout.rebuild(&window.root);
        self.reconcile_size(renderer);
        self.request_redraw(renderer);
    }

    /// Handles a layer-surface configure with the compositor's suggested logical size.
    pub fn configure(&mut self, renderer: &mut Renderer, (width, height): (u32, u32)) {
        // 0 means "pick your own size": use what was requested.
        let width = if width == 0 { self.requested.0 } else { width };
        let height = if height == 0 { self.requested.1 } else { height };
        let changed = !self.configured || (width, height) != self.size;
        self.size = (width, height);
        self.configured = true;

        // A newly known Fill axis can change how much a Fit axis needs.
        self.reconcile_size(renderer);
        if changed {
            self.dirty = true;
        }
        self.draw_if_ready(renderer);
    }

    pub fn set_scale(&mut self, renderer: &mut Renderer, scale: i32) {
        if scale == self.scale {
            return;
        }
        self.scale = scale;
        // Double-buffered: applied by the commit that presents the next (rescaled) frame.
        self.layer.wl_surface().set_buffer_scale(scale);
        self.request_redraw(renderer);
    }

    /// Called when the compositor fires this window's frame callback: the moment to draw
    /// the next frame, and so to advance any running animation to it.
    pub fn frame_done(&mut self, renderer: &mut Renderer, env: &Env) {
        self.frame_pending = false;
        if self.animating {
            self.rebuild(renderer, env);
        } else {
            self.draw_if_ready(renderer);
        }
    }

    /// A deadline to call [`WindowState::woken`] at, if one is newly wanted.
    pub fn arm_wake(&mut self) -> Option<Instant> {
        let wake = self.wake.filter(|_| !self.wake_armed)?;
        self.wake_armed = true;
        Some(wake)
    }

    /// Called at a deadline [`WindowState::arm_wake`] gave: builds the view again if its
    /// time has come.
    pub fn woken(&mut self, renderer: &mut Renderer, env: &Env) {
        self.wake_armed = false;
        if self.wake.is_none_or(|wake| wake > Instant::now()) {
            // Built again since, and so no longer waiting or waiting for a later time.
            return;
        }
        self.wake = None;
        if self.frame_pending {
            // The last frame has yet to be shown (and won't be while the window is hidden):
            // build the next when it has been.
            self.animating = true;
        } else {
            self.rebuild(renderer, env);
        }
    }

    fn request_redraw(&mut self, renderer: &mut Renderer) {
        self.dirty = true;
        self.draw_if_ready(renderer);
    }

    /// Re-measures the window. A changed size request needs a commit, which the next
    /// drawn frame provides.
    fn reconcile_size(&mut self, renderer: &mut Renderer) {
        if self.update_requested_size(renderer) {
            self.dirty = true;
        }
    }

    /// Computes the size to request from the compositor and sends it if it changed.
    fn update_requested_size(&mut self, renderer: &mut Renderer) -> bool {
        let (w, h) = (self.options.width, self.options.height);
        // Constraints for measuring: known sizes on fixed/filled axes, content on Fit axes.
        let known = |extent: Extent, configured: u32| match extent {
            Extent::Fixed(px) => Some(px as f32),
            Extent::Fill if self.configured && configured > 0 => Some(configured as f32),
            Extent::Fill | Extent::Fit => None,
        };
        let measured = if w == Extent::Fit || h == Extent::Fit {
            let (kw, kh) = (known(w, self.size.0), known(h, self.size.1));
            self.layout.compute(renderer.fonts().system(), kw, kh)
        } else {
            (0.0, 0.0)
        };
        let resolve = |extent: Extent, measured: f32| match extent {
            Extent::Fill => 0,
            Extent::Fixed(px) => px,
            // Layer surfaces must have a non-zero size on unanchored axes.
            Extent::Fit => (measured.ceil() as u32).max(1),
        };
        let requested = (resolve(w, measured.0), resolve(h, measured.1));
        if requested == self.requested {
            return false;
        }
        self.requested = requested;
        self.layer.set_size(requested.0, requested.1);
        self.layer.set_exclusive_zone(self.options.exclusive_zone_for(requested));
        true
    }

    fn draw_if_ready(&mut self, renderer: &mut Renderer) {
        let (width, height) = self.size;
        if !self.dirty || self.frame_pending || !self.configured || width == 0 || height == 0 {
            return;
        }

        let scale = self.scale as u32;
        renderer.resize(&mut self.target, width * scale, height * scale);

        self.layout.compute(renderer.fonts().system(), Some(width as f32), Some(height as f32));
        let mut scene = Scene::new(self.background);
        self.hits = self.layout.paint(&mut scene);

        let surface = self.layer.wl_surface();
        let qh = &self.qh;
        let presented = renderer.render(&mut self.target, &scene, self.scale as f32, || {
            // Rides along with the commit performed by present.
            surface.frame(qh, surface.clone());
        });
        if presented {
            self.dirty = false;
            self.frame_pending = true;
        }
        // If nothing was presented the window stays dirty and is retried on the next event.
    }
}
