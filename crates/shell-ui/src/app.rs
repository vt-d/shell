//! The UI runtime: owns the Wayland connection, the renderer, and every open window,
//! and routes compositor events to the window they concern.

use std::{collections::HashMap, error::Error};

use shell_render::Renderer;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            EventLoop, LoopHandle, LoopSignal,
            timer::{TimeoutAction, Timer},
        },
        calloop_wayland_source::WaylandSource,
        protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
            Shape as CursorShape, WpCursorShapeDeviceV1,
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent as SeatKeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{
            BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, PointerEvent as SeatPointerEvent, PointerEventKind,
            PointerHandler, cursor_shape::CursorShapeManager,
        },
    },
    shell::{
        WaylandSurface,
        wlr_layer::{LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
    },
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{
        wl_keyboard::WlKeyboard, wl_output, wl_pointer::WlPointer, wl_seat::WlSeat,
        wl_surface::WlSurface,
    },
};

use crate::{
    env::Env,
    layout::Hit,
    window::{View, WindowState},
};

/// Identifies an open window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(u64);

/// The shell's UI state. It is the data of the calloop event loop it was connected to:
/// every event source on that loop gets `&mut Ui`, so timers, IPC and the like can
/// update windows directly.
pub struct Ui {
    conn: Connection,
    qh: QueueHandle<Ui>,
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    pointer: Option<WlPointer>,
    /// For asking the compositor for a cursor, where it can be asked.
    cursor_shapes: Option<CursorShapeManager>,
    cursor: Option<WpCursorShapeDeviceV1>,
    /// The serial of the pointer's last entry into a window, which setting a cursor needs.
    enter_serial: u32,
    /// The window the pointer is over, and the target in it.
    hover: Option<(WindowId, Option<Hit>)>,
    /// The window the pointer was last in, and where in it.
    pointer_at: Option<(WindowId, f32, f32)>,
    /// What the left button went down on, until it is released.
    press: Option<(WindowId, Option<Hit>)>,
    /// `None` while a handler is being called.
    pointer_handlers: HashMap<WindowId, Option<PointerCallback>>,
    keyboard: Option<WlKeyboard>,
    /// The window the keyboard types into, if it is one of ours.
    focus: Option<WindowId>,
    modifiers: Modifiers,
    /// `None` while a handler is being called.
    key_handlers: HashMap<WindowId, Option<KeyCallback>>,
    compositor: CompositorState,
    layer_shell: LayerShell,
    loop_signal: LoopSignal,
    loop_handle: LoopHandle<'static, Ui>,
    // Declared before `renderer` so surfaces drop before the GPU device.
    windows: Vec<(WindowId, WindowState)>,
    renderer: Option<Renderer>,
    /// Fonts [added](Ui::add_font), loaded into the renderer once it exists.
    fonts: Vec<&'static [u8]>,
    sans_serif: Option<&'static str>,
    env: Env,
    next_id: u64,
}

impl Ui {
    /// Connects to the compositor and registers the Wayland connection on `event_loop`.
    pub fn connect(event_loop: &EventLoop<'static, Ui>) -> Result<Self, Box<dyn Error>> {
        let conn = Connection::connect_to_env()?;
        let (globals, event_queue) = registry_queue_init(&conn)?;
        let qh = event_queue.handle();

        let compositor = CompositorState::bind(&globals, &qh)?;
        let layer_shell = LayerShell::bind(&globals, &qh)?;
        WaylandSource::new(conn.clone(), event_queue)
            .insert(event_loop.handle())
            .map_err(|e| e.error)?;

        Ok(Self {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            seat_state: SeatState::new(&globals, &qh),
            pointer: None,
            // Without it the cursor keeps whatever shape it came in with.
            cursor_shapes: CursorShapeManager::bind(&globals, &qh).ok(),
            cursor: None,
            enter_serial: 0,
            pointer_at: None,
            hover: None,
            press: None,
            pointer_handlers: HashMap::new(),
            keyboard: None,
            focus: None,
            modifiers: Modifiers::default(),
            key_handlers: HashMap::new(),
            conn,
            qh,
            compositor,
            layer_shell,
            loop_signal: event_loop.get_signal(),
            loop_handle: event_loop.handle(),
            windows: Vec::new(),
            renderer: None,
            fonts: Vec::new(),
            sans_serif: None,
            env: Env::default(),
            next_id: 0,
        })
    }

    /// The event loop the UI runs on, for adding event sources that update it.
    pub fn loop_handle(&self) -> &LoopHandle<'static, Ui> {
        &self.loop_handle
    }

    /// Makes `value` available to every view, through [`Frame::get`], replacing any
    /// value of the same type provided before. If that changes it, every open window's
    /// view is built again. Use it for what all windows depend on, such as a theme.
    ///
    /// [`Frame::get`]: crate::Frame::get
    pub fn provide<T: PartialEq + 'static>(&mut self, value: T) {
        if !self.env.set(value) {
            return;
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        for (_, window) in &mut self.windows {
            window.rebuild(renderer, &self.env);
        }
        self.reap();
    }

    /// Adds a font (a TrueType/OpenType file's contents, typically `include_bytes!`ed) to
    /// those text can be set in, alongside the system's. Call it before opening windows.
    pub fn add_font(&mut self, data: &'static [u8]) {
        self.fonts.push(data);
        if let Some(renderer) = &mut self.renderer {
            renderer.fonts().add(data);
        }
    }

    /// Sets text in `family` unless it asks for another
    /// ([`FontFamily::SansSerif`](crate::FontFamily::SansSerif), the default, becomes it).
    /// Call it before opening windows.
    pub fn set_default_font(&mut self, family: &'static str) {
        self.sans_serif = Some(family);
        if let Some(renderer) = &mut self.renderer {
            renderer.fonts().set_sans_serif(family);
        }
    }

    /// The value of type `T` last [provided](Ui::provide), if any.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.env.get()
    }

    /// Opens a window. `view` builds its description, and is built again whenever a
    /// [provided](Ui::provide) value changes or [`Ui::refresh`] is called; a plain
    /// [`Window`](crate::Window) works too, for windows that never change. The window
    /// appears once the compositor has configured it.
    ///
    /// ```ignore
    /// ui.open(|frame: &mut Frame| {
    ///     Window::layer_shell("bar").background(Color::hex(0x1e1e2e))
    /// });
    /// ```
    pub fn open(&mut self, view: impl View) -> WindowId {
        let wl_surface = self.compositor.create_surface(&self.qh);
        // The GPU context is created with the first surface, so the adapter is one that
        // can present to it.
        let (renderer, target) = match &mut self.renderer {
            Some(renderer) => {
                let target = renderer.create_surface(&wl_surface);
                (renderer, target)
            }
            None => {
                let (mut renderer, target) = Renderer::new(&self.conn, &wl_surface);
                for &font in &self.fonts {
                    renderer.fonts().add(font);
                }
                if let Some(family) = self.sans_serif {
                    renderer.fonts().set_sans_serif(family);
                }
                (self.renderer.insert(renderer), target)
            }
        };
        let state = WindowState::new(
            Box::new(view),
            &self.layer_shell,
            &self.qh,
            wl_surface,
            target,
            renderer,
            &self.env,
        );

        let id = WindowId(self.next_id);
        self.next_id += 1;
        self.windows.push((id, state));
        self.arm_wakes();
        id
    }

    /// Destroys a window. The event loop stops once no windows remain.
    pub fn close(&mut self, id: WindowId) {
        self.windows.retain(|(window_id, _)| *window_id != id);
        self.pointer_handlers.remove(&id);
        self.key_handlers.remove(&id);
        self.focus.take_if(|window| *window == id);
        self.hover.take_if(|(window, _)| *window == id);
        self.press.take_if(|(window, _)| *window == id);
        if self.windows.is_empty() {
            self.loop_signal.stop();
        }
    }

    /// Closes the windows whose views asked to be closed.
    /// Sets a timer for each window whose view asked to be built again after a while.
    fn arm_wakes(&mut self) {
        for (id, window) in &mut self.windows {
            let Some(deadline) = window.arm_wake() else { continue };
            let id = *id;
            let timer = Timer::from_deadline(deadline);
            let result = self.loop_handle.insert_source(timer, move |_, _, ui| {
                ui.woken(id);
                TimeoutAction::Drop
            });
            if let Err(err) = result {
                eprintln!("ui: can't set a timer: {err}");
            }
        }
    }

    fn woken(&mut self, id: WindowId) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        if let Some((_, window)) = self.windows.iter_mut().find(|(window_id, _)| *window_id == id) {
            window.woken(renderer, &self.env);
        }
        self.reap();
    }

    fn reap(&mut self) {
        let done: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, window)| window.close_requested())
            .map(|(id, _)| *id)
            .collect();
        for id in done {
            self.close(id);
        }
        self.arm_wakes();
    }

    /// Rebuilds a window's view, for when something it shows has changed (a clock
    /// ticking, say). Its content is re-laid out (and resized, for [`Extent::Fit`] axes)
    /// and redrawn on the next frame.
    ///
    /// [`Extent::Fit`]: crate::Extent::Fit
    pub fn refresh(&mut self, id: WindowId) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        if let Some((_, window)) = self.windows.iter_mut().find(|(window_id, _)| *window_id == id) {
            window.rebuild(renderer, &self.env);
        }
        self.reap();
    }

    /// The window drawing to `surface`, along with the renderer to draw it with.
    fn window_for(&mut self, surface: &WlSurface) -> Option<(&mut WindowState, &mut Renderer)> {
        let renderer = self.renderer.as_mut()?;
        let (_, window) = self.windows.iter_mut().find(|(_, w)| w.wl_surface() == surface)?;
        Some((window, renderer))
    }
}

impl LayerShellHandler for Ui {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        let closed = self.windows.iter().find(|(_, w)| w.wl_surface() == layer.wl_surface());
        if let Some(&(id, _)) = closed {
            self.close(id);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if let Some((window, renderer)) = self.window_for(layer.wl_surface()) {
            window.configure(renderer, configure.new_size);
        }
    }
}

impl CompositorHandler for Ui {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &WlSurface,
        new_factor: i32,
    ) {
        if let Some((window, renderer)) = self.window_for(surface) {
            window.set_scale(renderer, new_factor);
        }
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &WlSurface, _time: u32) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        if let Some((_, window)) = self.windows.iter_mut().find(|(_, w)| w.wl_surface() == surface)
        {
            window.frame_done(renderer, &self.env);
        }
        self.reap();
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Ui {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for Ui {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Ui);
delegate_output!(Ui);
delegate_layer!(Ui);
delegate_registry!(Ui);
delegate_seat!(Ui);
delegate_pointer!(Ui);
delegate_keyboard!(Ui);

/// What the pointer did, and to which [target](crate::Element::target).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    /// The target it happened on; `None` for a press, release or click on no target.
    pub target: Option<u32>,
    pub kind: PointerKind,
    /// Where the pointer is, from the target's top-left corner (or the window's, with no
    /// target), in logical pixels. While dragging it may be outside the target.
    pub x: f32,
    pub y: f32,
    /// The target's size (or zero, with no target).
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerKind {
    /// The pointer came over the target.
    Enter,
    /// It left it.
    Leave,
    /// The left button went down on it. Until it's released, the target keeps the
    /// pointer: it gets [`PointerKind::Drag`]s wherever the pointer goes.
    Press,
    /// The pointer moved with the button held.
    Drag,
    /// The button came up.
    Release,
    /// It came up over the target it went down on. Follows the `Release`.
    Click,
    /// The right button went down on the target: what asks for a menu.
    SecondaryPress,
    /// The middle button went down on it.
    MiddlePress,
    /// The wheel turned (or two fingers moved) over the target, by this many steps;
    /// positive is downward.
    Scroll(f32),
}

type PointerCallback = Box<dyn FnMut(&mut Ui, PointerEvent)>;

/// How far a touchpad scrolls for one step of a wheel, in pixels.
const SCROLL_STEP: f64 = 15.0;

impl Ui {
    /// Calls `handler` with what the pointer does on window `id`'s
    /// [targets](crate::Element::target) (and with presses and clicks on none of them),
    /// replacing any handler set before. It gets the UI, to refresh the window or open
    /// and close others.
    pub fn on_pointer(
        &mut self,
        id: WindowId,
        handler: impl FnMut(&mut Ui, PointerEvent) + 'static,
    ) {
        self.pointer_handlers.insert(id, Some(Box::new(handler)));
    }

    /// Where the pointer last was: the window, and the place in it in logical pixels
    /// from its top-left corner. For placing what a click opens.
    pub fn pointer_position(&self) -> Option<(WindowId, f32, f32)> {
        self.pointer_at
    }

    /// Window `id`'s size in logical pixels, once the compositor has given it one.
    pub fn window_size(&self, id: WindowId) -> Option<(u32, u32)> {
        let (_, window) = self.windows.iter().find(|(window_id, _)| *window_id == id)?;
        Some(window.size()).filter(|&(width, height)| width > 0 && height > 0)
    }

    /// Reports `kind` at `position` (in the window) to window `id`'s handler.
    fn emit(&mut self, id: WindowId, hit: Option<Hit>, kind: PointerKind, position: (f32, f32)) {
        let Some(mut handler) = self.pointer_handlers.get_mut(&id).and_then(Option::take) else {
            return;
        };
        let event = match hit {
            Some(hit) => PointerEvent {
                target: Some(hit.id),
                kind,
                x: position.0 - hit.rect.x,
                y: position.1 - hit.rect.y,
                width: hit.rect.width,
                height: hit.rect.height,
            },
            None => PointerEvent {
                target: None,
                kind,
                x: position.0,
                y: position.1,
                width: 0.0,
                height: 0.0,
            },
        };
        handler(self, event);
        // Unless the handler closed the window or set another handler.
        if let Some(slot @ None) = self.pointer_handlers.get_mut(&id) {
            *slot = Some(handler);
        }
    }

    /// Notes what the pointer is over now, reporting the target it left and the one it
    /// came over, and showing a hand over targets.
    fn set_hover(&mut self, hover: Option<(WindowId, Option<Hit>)>, position: (f32, f32)) {
        let target = |hover: &Option<(WindowId, Option<Hit>)>| {
            hover.as_ref().and_then(|(id, hit)| hit.map(|hit| (*id, hit.id)))
        };
        let (old, new) = (target(&self.hover), target(&hover));
        let previous = std::mem::replace(&mut self.hover, hover);
        if old == new {
            return;
        }
        if let Some((id, hit @ Some(_))) = previous {
            self.emit(id, hit, PointerKind::Leave, position);
        }
        if let Some((id, hit @ Some(_))) = hover {
            self.emit(id, hit, PointerKind::Enter, position);
        }
        if let Some(cursor) = &self.cursor {
            let shape = if new.is_some() { CursorShape::Pointer } else { CursorShape::Default };
            cursor.set_shape(self.enter_serial, shape);
        }
    }
}

impl SeatHandler for Ui {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            // A held key repeats from a timer on the loop.
            let keyboard = self.seat_state.get_keyboard_with_repeat(
                qh,
                &seat,
                None,
                self.loop_handle.clone(),
                Box::new(|ui: &mut Ui, _, event| ui.key(event)),
            );
            match keyboard {
                Ok(keyboard) => self.keyboard = Some(keyboard),
                Err(err) => eprintln!("can't use the keyboard: {err}"),
            }
        }
        if capability != Capability::Pointer || self.pointer.is_some() {
            return;
        }
        match self.seat_state.get_pointer(qh, &seat) {
            Ok(pointer) => {
                self.cursor =
                    self.cursor_shapes.as_ref().map(|shapes| shapes.get_shape_device(&pointer, qh));
                self.pointer = Some(pointer);
            }
            Err(err) => eprintln!("can't use the pointer: {err}"),
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
            self.focus = None;
        }
        if capability == Capability::Pointer {
            if let Some(cursor) = self.cursor.take() {
                cursor.destroy();
            }
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
            self.hover = None;
            self.press = None;
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl PointerHandler for Ui {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlPointer,
        events: &[SeatPointerEvent],
    ) {
        for event in events {
            let window = self.windows.iter().find(|(_, w)| w.wl_surface() == &event.surface);
            let Some((id, window)) = window else { continue };
            let id = *id;
            let position = (event.position.0 as f32, event.position.1 as f32);
            self.pointer_at = Some((id, position.0, position.1));
            let hit = window.hit(position.0, position.1);
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    self.enter_serial = serial;
                    // So the cursor is set even if it comes in over no target.
                    if let Some(cursor) = &self.cursor {
                        cursor.set_shape(serial, CursorShape::Default);
                    }
                    self.set_hover(Some((id, hit)), position);
                }
                PointerEventKind::Leave { .. } => self.set_hover(None, position),
                PointerEventKind::Motion { .. } => match self.press {
                    // What was pressed keeps the pointer until it's let go.
                    Some((pressed, held)) => {
                        if held.is_some() {
                            self.emit(pressed, held, PointerKind::Drag, position);
                        }
                    }
                    None => self.set_hover(Some((id, hit)), position),
                },
                PointerEventKind::Press { button: BTN_LEFT, .. } => {
                    self.press = Some((id, hit));
                    self.emit(id, hit, PointerKind::Press, position);
                }
                PointerEventKind::Press { button: BTN_RIGHT, .. } if hit.is_some() => {
                    self.emit(id, hit, PointerKind::SecondaryPress, position);
                }
                PointerEventKind::Press { button: BTN_MIDDLE, .. } if hit.is_some() => {
                    self.emit(id, hit, PointerKind::MiddlePress, position);
                }
                PointerEventKind::Release { button: BTN_LEFT, .. } => {
                    let Some((pressed, held)) = self.press.take() else { continue };
                    self.emit(pressed, held, PointerKind::Release, position);
                    let over = match held {
                        Some(held) => hit.is_some_and(|hit| hit.id == held.id),
                        None => hit.is_none(),
                    };
                    if pressed == id && over {
                        self.emit(pressed, held, PointerKind::Click, position);
                    }
                    // The handlers may have changed what's under the pointer, or closed
                    // the window.
                    let window = self.windows.iter().find(|(window, _)| *window == id);
                    let hover = window.map(|(_, w)| (id, w.hit(position.0, position.1)));
                    self.set_hover(hover, position);
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let steps = if vertical.discrete != 0 {
                        vertical.discrete as f32
                    } else {
                        (vertical.absolute / SCROLL_STEP) as f32
                    };
                    if hit.is_some() && steps != 0.0 {
                        self.emit(id, hit, PointerKind::Scroll(steps), position);
                    }
                }
                _ => {}
            }
        }
    }
}

/// A key going down (or repeating, held) in a window with the keyboard's focus.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyEvent {
    pub key: Key,
    /// What it types, if anything: for text fields. Nothing with Ctrl or Alt held.
    pub text: Option<String>,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Escape,
    Enter,
    Tab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// A key that types this character, in lower case and whatever is held with it: for
    /// shortcuts such as Ctrl+U.
    Char(char),
    Other,
}

impl Key {
    fn from_keysym(keysym: Keysym) -> Self {
        match keysym {
            Keysym::Escape => Self::Escape,
            Keysym::Return | Keysym::KP_Enter => Self::Enter,
            Keysym::Tab | Keysym::ISO_Left_Tab => Self::Tab,
            Keysym::BackSpace => Self::Backspace,
            Keysym::Delete => Self::Delete,
            Keysym::Up => Self::Up,
            Keysym::Down => Self::Down,
            Keysym::Left => Self::Left,
            Keysym::Right => Self::Right,
            Keysym::Home => Self::Home,
            Keysym::End => Self::End,
            Keysym::Page_Up => Self::PageUp,
            Keysym::Page_Down => Self::PageDown,
            keysym => match keysym.key_char() {
                Some(c) if !c.is_control() => Self::Char(c.to_lowercase().next().unwrap_or(c)),
                _ => Self::Other,
            },
        }
    }
}

type KeyCallback = Box<dyn FnMut(&mut Ui, KeyEvent)>;

impl Ui {
    /// Calls `handler` with the keys pressed while window `id` has the keyboard's focus,
    /// replacing any handler set before. A window only gets the focus if it
    /// [asks for the keyboard](crate::Window::keyboard).
    pub fn on_key(&mut self, id: WindowId, handler: impl FnMut(&mut Ui, KeyEvent) + 'static) {
        self.key_handlers.insert(id, Some(Box::new(handler)));
    }

    /// Reports a key press to the focused window's handler.
    fn key(&mut self, event: SeatKeyEvent) {
        let Some(id) = self.focus else { return };
        let Some(mut handler) = self.key_handlers.get_mut(&id).and_then(Option::take) else {
            return;
        };
        let Modifiers { ctrl, alt, shift, .. } = self.modifiers;
        let text = event.utf8.filter(|text| {
            !ctrl && !alt && !text.is_empty() && !text.chars().any(char::is_control)
        });
        handler(self, KeyEvent { key: Key::from_keysym(event.keysym), text, ctrl, alt, shift });
        // Unless the handler closed the window or set another handler.
        if let Some(slot @ None) = self.key_handlers.get_mut(&id) {
            *slot = Some(handler);
        }
    }
}

impl KeyboardHandler for Ui {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        surface: &WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
        self.focus =
            self.windows.iter().find(|(_, w)| w.wl_surface() == surface).map(|(id, _)| *id);
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: &WlSurface,
        _serial: u32,
    ) {
        self.focus = None;
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _serial: u32,
        event: SeatKeyEvent,
    ) {
        self.key(event);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _serial: u32,
        _: SeatKeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _layout: u32,
    ) {
        self.modifiers = modifiers;
    }
}
