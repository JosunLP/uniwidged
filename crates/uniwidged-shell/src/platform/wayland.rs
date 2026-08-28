//! The widget's surface on Wayland.
//!
//! Adapted from Ephemeris `src/unix/linux/wayland/window.rs`, and promoted out
//! of `spikes/m0_layer_shell` — which is where the load-bearing question was
//! answered: a `zwlr_layer_shell_v1` surface on the `BOTTOM` layer really does
//! back a wgpu swapchain with per-pixel alpha.
//!
//! **`wlr-layer-shell` is the whole point.** It is the only Wayland protocol
//! that lets a client say "above the wallpaper, below every ordinary window",
//! which is what a desktop widget is defined by. On a compositor that has it —
//! Sway, Hyprland, river, Wayfire, KDE Plasma — the surface goes on the
//! `BOTTOM` layer with `keyboard-interactivity` off and behaves exactly as the
//! Win32 and AppKit surfaces do.
//!
//! **Where there is none, the widget says so.** GNOME's Mutter does not
//! implement the layer shell and has stated it will not. There the surface
//! falls back to an ordinary `xdg_toplevel` and two things change: it sits
//! among the windows rather than behind them, and it cannot place itself,
//! because Wayland gives no client that power. Both go in the log at start-up,
//! and [`Shape::capabilities`] is what says so in code.
//!
//! ## Three ways this is not X11
//!
//! The host above this file is the same, so the differences all live here.
//!
//! 1. **A client cannot place a window.** There is no `ConfigureWindow`. A
//!    layer surface is *anchored* to an edge and pushed away from it by
//!    margins, so [`WaylandWindow::set_geometry`] is an anchor and four
//!    margins rather than an x and a y. The result is identical and the
//!    mechanism is not.
//!
//! 2. **A client is never told where the pointer is.** Wayland reports
//!    surface-local coordinates only — deliberately, so that a client cannot
//!    watch the pointer cross somebody else's window. The desktop coordinates
//!    `uniwidged_host::interaction` wants are therefore *computed* from the
//!    placement this file asked for, which it knows exactly because it set it.
//!
//! 3. **The system libwayland, not the pure-Rust backend.** Mesa's Vulkan WSI
//!    drives the same `wl_display`, and the two have to be holding the same C
//!    object. A Rust connection has no pointer to give wgpu. This is not a
//!    detail that can be changed later; it decides which backend the whole
//!    crate uses.
//!
//! ## The lifecycle, which is the delicate part
//!
//! A layer surface must `ack_configure` before its first commit, and the size
//! it is given comes from the compositor. wgpu's surface API assumes it owns
//! configure and present timing, so the sequence is driven from here with wgpu
//! attached to it, not the other way round — see `ARCHITECTURE.md` §10.2.
//! [`WaylandWindow::open`] returns with the surface configured and **no buffer
//! attached**, which is exactly the state wgpu needs to find it in.

use std::ffi::c_void;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::ptr::NonNull;
use std::time::Duration;

use uniwidged_core::geom::PhysicalRect;
use wayland_client::protocol::{
    wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::{Capabilities, Input, Output, Placement};

/// The Linux button code for the primary button. Wayland passes evdev codes
/// straight through rather than numbering buttons itself.
const BTN_LEFT: u32 = 0x110;

/// How the surface ended up being shown — the one thing a compositor may not
/// let the widget choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `zwlr_layer_shell_v1`, `BOTTOM` layer. What a widget is meant to be.
    Layer,
    /// An ordinary `xdg_toplevel`, because the compositor has no layer shell.
    Toplevel,
}

impl Shape {
    pub fn capabilities(self) -> Capabilities {
        match self {
            Shape::Layer => Capabilities::FULL,
            Shape::Toplevel => Capabilities {
                below_windows: false,
                self_placing: false,
                ..Capabilities::FULL
            },
        }
    }
}

#[derive(Debug)]
pub enum WaylandError {
    /// No display to connect to, or the socket refused.
    Connect(String),
    /// The compositor is missing something a widget cannot do without.
    Missing(&'static str),
    Protocol(String),
    /// The compositor never completed the configure handshake.
    NotConfigured,
}

impl std::fmt::Display for WaylandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WaylandError::Connect(why) => write!(f, "no Wayland display: {why}"),
            WaylandError::Missing(what) => write!(f, "this compositor has no {what}"),
            WaylandError::Protocol(why) => write!(f, "{why}"),
            WaylandError::NotConfigured => {
                write!(f, "the compositor never sent a configure for the surface")
            }
        }
    }
}

impl std::error::Error for WaylandError {}

/// One monitor as the compositor describes it, while its events are arriving.
///
/// A `wl_output` is described by several events and is only complete at
/// `done`, so this accumulates and [`WaylandWindow::outputs`] reads it.
#[derive(Default, Clone)]
struct OutputState {
    name: Option<String>,
    position: (i32, i32),
    size: (u32, u32),
}

/// Everything the event callbacks write into.
///
/// Events arrive with no context of their own, so every handler does one
/// thing: record. Reaching back into the host from a callback would re-enter
/// code the loop itself called.
#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    outputs: Vec<(wl_output::WlOutput, OutputState)>,

    /// Set once the compositor has answered the first commit.
    configured: bool,
    /// The size it decided on. Zero means "you choose", which is what a layer
    /// surface says when it was given an explicit one.
    size: (u32, u32),
    closed: bool,

    /// Where this client has placed its own surface, so that surface-local
    /// pointer coordinates can be turned into desktop ones. See the module
    /// note: Wayland will not tell us, and we do not need telling.
    origin: (i32, i32),
    /// The surface the pointer is currently over, and where on it.
    pointer_on: Option<u32>,
    pointer_at: (f64, f64),
    /// What the loop will drain.
    pending: Vec<Input>,
}

impl State {
    /// Surface-local to desktop coordinates.
    fn root(&self) -> (i32, i32) {
        (
            self.origin.0 + self.pointer_at.0 as i32,
            self.origin.1 + self.pointer_at.1 as i32,
        )
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_compositor" => {
                state.compositor = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "zwlr_layer_shell_v1" => {
                state.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "xdg_wm_base" => {
                state.wm_base = Some(registry.bind(name, version.min(3), qh, ()));
            }
            "wl_seat" if state.seat.is_none() => {
                state.seat = Some(registry.bind(name, version.min(7), qh, ()));
            }
            "wl_output" => {
                // Version 4 is where an output gained a `name` event, and a
                // name is the only identifier a placement can be stored
                // against — see `uniwidged_host::placement`. On an older
                // compositor the name stays `None` and placement falls back to
                // desktop coordinates, which is the documented behaviour
                // rather than a guess.
                let output: wl_output::WlOutput = registry.bind(name, version.min(4), qh, ());
                state.outputs.push((output, OutputState::default()));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, info)) = state.outputs.iter_mut().find(|(known, _)| known == output) else {
            return;
        };
        match event {
            wl_output::Event::Geometry { x, y, .. } => info.position = (x, y),
            wl_output::Event::Mode { width, height, .. } => {
                info.size = (width.max(0) as u32, height.max(0) as u32)
            }
            wl_output::Event::Name { name } => info.name = Some(name),
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // A seat announces what it has, and may announce it again when
        // hardware is plugged in. Binding twice would give two pointers
        // reporting the same events.
        if let wl_seat::Event::Capabilities {
            capabilities: wayland_client::WEnum::Value(capabilities),
        } = event
            && capabilities.contains(wl_seat::Capability::Pointer)
            && state.pointer.is_none()
        {
            state.pointer = Some(seat.get_pointer(qh, ()));
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_on = Some(surface.id().protocol_id());
                state.pointer_at = (surface_x, surface_y);
            }
            wl_pointer::Event::Leave { .. } => {
                // Not a release. A pointer that leaves during a drag is still
                // holding the button, and the compositor keeps delivering to
                // us because of the implicit grab — so the gesture continues,
                // which is what the host expects and what X11 needs an
                // explicit grab to achieve.
                state.pointer_on = None;
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_at = (surface_x, surface_y);
                if let Some(window) = state.pointer_on {
                    state.pending.push(Input::Motion {
                        window,
                        root: state.root(),
                    });
                }
            }
            wl_pointer::Event::Button {
                button,
                state: button_state,
                ..
            } if button == BTN_LEFT => {
                let Some(window) = state.pointer_on else {
                    return;
                };
                let root = state.root();
                let local = (state.pointer_at.0 as i32, state.pointer_at.1 as i32);
                match button_state {
                    wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed) => {
                        state.pending.push(Input::Pressed {
                            window,
                            local,
                            root,
                        })
                    }
                    wayland_client::WEnum::Value(wl_pointer::ButtonState::Released) => {
                        state.pending.push(Input::Released { window, root })
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                // The handshake that must complete before a buffer may be
                // attached, and therefore before wgpu may present.
                layer_surface.ack_configure(serial);
                if width != 0 && height != 0 {
                    state.size = (width, height);
                }
                state.configured = true;
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.closed = true;
                state.configured = false;
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Not answering this is how a client gets killed for being
        // unresponsive, which looks exactly like a rendering bug.
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for State {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } if width > 0 && height > 0 => {
                state.size = (width as u32, height as u32);
            }
            xdg_toplevel::Event::Close => state.closed = true,
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

/// Which shell to ask for.
///
/// A parameter rather than an environment variable, because a test that has to
/// set one to reach a branch cannot run beside another test — and because the
/// choice belongs to the caller either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Prefer {
    /// `zwlr_layer_shell_v1` where the compositor has it, and the
    /// `xdg_toplevel` fallback where it does not.
    #[default]
    LayerShell,
    /// The fallback, whatever the compositor offers. What a GNOME session gets
    /// anyway, and the only way to exercise that path deliberately.
    Toplevel,
}

/// A configured desktop-layer surface.
pub struct WaylandWindow {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    surface: wl_surface::WlSurface,
    /// Kept alive: dropping the role object destroys the role.
    layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    _xdg: Option<(xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel)>,
    shape: Shape,
    placement: Placement,
}

impl WaylandWindow {
    /// Connects to `$WAYLAND_DISPLAY` and creates a widget surface.
    pub fn open(rect: PhysicalRect, title: &str) -> Result<Self, WaylandError> {
        Self::open_on(None, rect, title, Prefer::LayerShell)
    }

    /// As [`WaylandWindow::open`], on a socket given by path and with the
    /// shell chosen rather than detected.
    ///
    /// `None` is `$WAYLAND_DISPLAY`, which is what a desktop wants. A path is
    /// for a test that starts its own compositor: setting `WAYLAND_DISPLAY`
    /// from inside a running process is a data race, and the compiler has said
    /// so since edition 2024.
    pub fn open_on(
        socket: Option<&Path>,
        rect: PhysicalRect,
        title: &str,
        prefer: Prefer,
    ) -> Result<Self, WaylandError> {
        let conn = match socket {
            Some(path) => {
                let stream = UnixStream::connect(path)
                    .map_err(|e| WaylandError::Connect(format!("{}: {e}", path.display())))?;
                Connection::from_socket(stream).map_err(|e| WaylandError::Connect(e.to_string()))?
            }
            None => Connection::connect_to_env()
                .map_err(|e| WaylandError::Connect(format!("{e} (is WAYLAND_DISPLAY set?)")))?,
        };

        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());

        let mut state = State::default();
        // Twice: the first brings the globals, the second the events those
        // globals then send — an output's name and mode arrive after it is
        // bound, and a seat announces its pointer the same way.
        queue
            .roundtrip(&mut state)
            .map_err(|e| WaylandError::Protocol(e.to_string()))?;
        queue
            .roundtrip(&mut state)
            .map_err(|e| WaylandError::Protocol(e.to_string()))?;

        let compositor = state
            .compositor
            .clone()
            .ok_or(WaylandError::Missing("wl_compositor, which is impossible"))?;
        let surface = compositor.create_surface(&qh, ());

        // Asking for the fallback is how the degraded path gets tested at all:
        // every compositor that has a layer shell always has one, so the branch
        // a GNOME user takes would otherwise never run on a machine that could
        // run it.
        let shell = match prefer {
            Prefer::LayerShell => state.layer_shell.clone(),
            Prefer::Toplevel => None,
        };

        let (shape, layer_surface, xdg) = match shell {
            Some(shell) => {
                use zwlr_layer_shell_v1::Layer;
                use zwlr_layer_surface_v1::KeyboardInteractivity;
                let layer_surface = shell.get_layer_surface(
                    &surface,
                    // No output: the compositor puts it on the one the user is
                    // looking at, and `set_geometry` moves it afterwards.
                    None,
                    // `Bottom`, not `Background`: background is where a
                    // wallpaper goes and it sits *under* the desktop's own
                    // surfaces. `Bottom` is above the wallpaper and below every
                    // ordinary window, which is the definition of a widget.
                    Layer::Bottom,
                    title.to_string(),
                    &qh,
                    (),
                );
                // A widget never takes the keyboard. This is the Wayland
                // spelling of `WS_EX_NOACTIVATE` and of `WM_HINTS.input =
                // False`.
                layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
                (Shape::Layer, Some(layer_surface), None)
            }
            None => {
                let wm_base = state.wm_base.clone().ok_or(WaylandError::Missing(
                    "layer shell and no xdg_wm_base either — nothing to make a surface with",
                ))?;
                let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
                let toplevel = xdg_surface.get_toplevel(&qh, ());
                toplevel.set_title(title.to_string());
                toplevel.set_app_id("dev.uniwidged.widget".into());
                state.size = (rect.width, rect.height);
                (Shape::Toplevel, None, Some((xdg_surface, toplevel)))
            }
        };

        let mut window = Self {
            conn,
            queue,
            state,
            surface,
            layer_surface,
            _xdg: xdg,
            shape,
            placement: Placement {
                rect,
                output: None,
                scale: 1.0,
            },
        };
        window.place(rect);

        // Commit with no buffer: this is what asks for the configure.
        window.surface.commit();
        window
            .queue
            .roundtrip(&mut window.state)
            .map_err(|e| WaylandError::Protocol(e.to_string()))?;
        if !window.state.configured {
            return Err(WaylandError::NotConfigured);
        }
        if window.state.size != (0, 0) {
            window.placement.rect.width = window.state.size.0;
            window.placement.rect.height = window.state.size.1;
        }
        window.state.origin = (window.placement.rect.x, window.placement.rect.y);
        Ok(window)
    }

    pub fn shape(&self) -> Shape {
        self.shape
    }

    pub fn capabilities(&self) -> Capabilities {
        self.shape.capabilities()
    }

    /// A Wayland session always composites. There is no mode in which it does
    /// not, which is why transparency is X11's risk and not this platform's.
    pub fn has_compositor(&self) -> bool {
        true
    }

    /// The surface's protocol id, which is what [`Input`] reports events
    /// against.
    pub fn window_id(&self) -> u32 {
        self.surface.id().protocol_id()
    }

    pub fn geometry(&self) -> PhysicalRect {
        self.placement.rect
    }

    /// The pointers wgpu needs. Valid as long as this window is.
    pub fn raw_handles(
        &self,
    ) -> (
        raw_window_handle::RawDisplayHandle,
        raw_window_handle::RawWindowHandle,
    ) {
        let display = self.conn.backend().display_ptr() as *mut c_void;
        let surface = self.surface.id().as_ptr() as *mut c_void;
        (
            raw_window_handle::RawDisplayHandle::Wayland(
                raw_window_handle::WaylandDisplayHandle::new(
                    NonNull::new(display).expect("a live connection has a display"),
                ),
            ),
            raw_window_handle::RawWindowHandle::Wayland(
                raw_window_handle::WaylandWindowHandle::new(
                    NonNull::new(surface).expect("a live surface has a proxy"),
                ),
            ),
        )
    }

    /// Moves and resizes the surface.
    ///
    /// There is no `ConfigureWindow` on Wayland and no way to ask for an
    /// absolute position. A layer surface is anchored to two edges and pushed
    /// away from them by margins, so a rectangle becomes `Top | Left` plus a
    /// top and a left margin — which produces exactly the requested position
    /// and is the reason this file can compute desktop coordinates for the
    /// pointer at all.
    ///
    /// On the `xdg_toplevel` fallback this changes the size and nothing else:
    /// an ordinary Wayland window cannot place itself, and pretending
    /// otherwise would leave the host's idea of where the widget is disagreeing
    /// with the screen.
    pub fn set_geometry(&mut self, rect: PhysicalRect) -> Result<(), WaylandError> {
        if rect == self.placement.rect {
            return Ok(());
        }
        self.place(rect);
        self.surface.commit();
        self.conn
            .flush()
            .map_err(|e| WaylandError::Protocol(e.to_string()))?;
        self.placement.rect = rect;
        // The pointer's desktop coordinates are derived from this, so it has to
        // move at the same moment or one drag step is measured against the old
        // position.
        self.state.origin = (rect.x, rect.y);
        Ok(())
    }

    fn place(&mut self, rect: PhysicalRect) {
        let Some(layer_surface) = &self.layer_surface else {
            return;
        };
        use zwlr_layer_surface_v1::Anchor;
        layer_surface.set_size(rect.width, rect.height);
        layer_surface.set_anchor(Anchor::Top | Anchor::Left);
        layer_surface.set_margin(rect.y, 0, 0, rect.x);
    }

    /// The monitors this compositor has, by name.
    ///
    /// Empty when it advertises `wl_output` below version 4, which is the
    /// version that added the name — and a name is the only identifier a
    /// placement can be stored against.
    pub fn outputs(&self) -> Vec<Output> {
        self.state
            .outputs
            .iter()
            .filter_map(|(_, info)| {
                let name = info.name.clone()?;
                Some(Output {
                    name,
                    rect: PhysicalRect::new(
                        info.position.0,
                        info.position.1,
                        info.size.0,
                        info.size.1,
                    ),
                    // Wayland has no notion of a primary output. Nothing here
                    // marks one, and `placement` falls back to the first —
                    // which is why that fallback exists.
                    primary: false,
                })
            })
            .collect()
    }

    /// Every monitor's union, in physical pixels.
    pub fn desktop_bounds(&self) -> PhysicalRect {
        let outputs = self.outputs();
        if outputs.is_empty() {
            return self.placement.rect;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for output in &outputs {
            x0 = x0.min(output.rect.x);
            y0 = y0.min(output.rect.y);
            x1 = x1.max(output.rect.x + output.rect.width as i32);
            y1 = y1.max(output.rect.y + output.rect.height as i32);
        }
        PhysicalRect::new(x0, y0, (x1 - x0).max(0) as u32, (y1 - y0).max(0) as u32)
    }

    /// The pointer is already routed to us for the duration of a press.
    ///
    /// Wayland has an *implicit* grab: from the moment a button goes down until
    /// it comes up, every pointer event goes to the surface it went down on,
    /// wherever the pointer travels. X11 needs `grab_pointer` to get the same
    /// behaviour; here it is the protocol's own guarantee, and asking for it
    /// would be asking for something that does not exist.
    pub fn grab_pointer(&self) -> Result<(), WaylandError> {
        Ok(())
    }

    pub fn release_pointer(&self) -> Result<(), WaylandError> {
        Ok(())
    }

    /// Blocks until something arrives or `timeout` elapses.
    ///
    /// `None` means block indefinitely, which is the common case on an idle
    /// desktop and is what makes it cost nothing. This is Wayland's half of
    /// `ARCHITECTURE.md` §5.
    pub fn wait(&mut self, timeout: Option<Duration>) -> Result<(), WaylandError> {
        self.wait_with(timeout, None)
    }

    /// As [`WaylandWindow::wait`], also waking when a background task
    /// finishes. See `crate::wake`.
    pub fn wait_with(
        &mut self,
        timeout: Option<Duration>,
        wake: Option<&crate::WakePipe>,
    ) -> Result<(), WaylandError> {
        use std::os::fd::AsRawFd;

        // Anything already dispatched means there is nothing to wait for.
        // Checking first also closes the race where an event arrived between
        // the last drain and this call, which would otherwise be slept through.
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(|e| WaylandError::Protocol(e.to_string()))?;
        if !self.state.pending.is_empty() {
            return Ok(());
        }
        self.conn
            .flush()
            .map_err(|e| WaylandError::Protocol(e.to_string()))?;

        // `prepare_read` returning `None` means the queue already has events,
        // which the dispatch below will take.
        let Some(guard) = self.conn.prepare_read() else {
            self.queue
                .dispatch_pending(&mut self.state)
                .map_err(|e| WaylandError::Protocol(e.to_string()))?;
            return Ok(());
        };
        let millis = match timeout {
            Some(d) => d.as_millis().min(i32::MAX as u128) as i32,
            None => -1,
        };
        let mut fds = vec![libc::pollfd {
            fd: guard.connection_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        if let Some(wake) = wake {
            fds.push(libc::pollfd {
                fd: wake.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
        // SAFETY: descriptors borrowed for the duration of the call and not
        // stored; the first from the guard, which outlives it.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, millis) };
        if ready > 0 && fds[0].revents != 0 {
            // An error here is a compositor that went away, which the next
            // dispatch reports properly.
            let _ = guard.read();
        } else {
            // Woken by a task rather than by the compositor. The guard has to
            // be cancelled or the next `prepare_read` sees a read still
            // outstanding and the loop stops receiving events entirely.
            drop(guard);
        }
        if let Some(wake) = wake {
            wake.drain();
        }
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(|e| WaylandError::Protocol(e.to_string()))?;
        Ok(())
    }

    /// Every event queued right now.
    pub fn drain_events(&mut self) -> Result<Vec<Input>, WaylandError> {
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(|e| WaylandError::Protocol(e.to_string()))?;
        let mut events = std::mem::take(&mut self.state.pending);
        if self.state.closed {
            self.state.closed = false;
            events.push(Input::Closed {
                window: self.window_id(),
            });
        }
        Ok(events)
    }

    /// Whether the compositor has resized the surface since this was last
    /// asked, and to what.
    ///
    /// A layer surface's size is the compositor's decision, not the client's —
    /// it may be smaller than requested on a small output. The caller has to
    /// resize its swapchain to match, which is why this is separate from
    /// [`WaylandWindow::drain_events`]: it is not something the *user* did.
    pub fn take_resize(&mut self) -> Option<(u32, u32)> {
        let size = self.state.size;
        if size == (0, 0) || size == (self.placement.rect.width, self.placement.rect.height) {
            return None;
        }
        self.placement.rect.width = size.0;
        self.placement.rect.height = size.1;
        Some(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toplevel_fallback_admits_what_it_lost() {
        assert!(Shape::Layer.capabilities().degradation().is_none());
        assert!(Shape::Toplevel.capabilities().degradation().is_some());
        assert!(Shape::Layer.capabilities().per_pixel_alpha);
        // The fallback gives up placement and stacking. It never gives up
        // transparency: a widget on an opaque rectangle is not a widget.
        assert!(Shape::Toplevel.capabilities().per_pixel_alpha);
    }

    #[test]
    fn every_error_says_something_a_person_could_act_on() {
        for error in [
            WaylandError::Connect("no such file".into()),
            WaylandError::Missing("wl_compositor"),
            WaylandError::Protocol("broken pipe".into()),
            WaylandError::NotConfigured,
        ] {
            let said = error.to_string();
            assert!(!said.is_empty() && said.len() > 10, "{said:?}");
        }
    }
}
