//! The Wayland half: get a surface that behaves like a desktop widget.
//!
//! Written against the system libwayland rather than the pure-Rust backend,
//! because Mesa's Vulkan WSI drives the same `wl_display` and the two have to
//! be holding the same C object. This is not a detail that can be fixed later;
//! it decides which backend the whole shell crate uses.

use std::ffi::c_void;
use std::ptr::NonNull;

use wayland_client::protocol::{wl_compositor, wl_output, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

/// Which role the surface ended up with — the one thing a compositor may not
/// let a widget choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `zwlr_layer_shell_v1` on the bottom layer. What a widget is meant to be.
    Layer,
    /// An ordinary toplevel, because the compositor has no layer shell.
    Toplevel,
}

impl Shape {
    /// What is lost, in the words the start-up log will use.
    pub fn degradation(self) -> Option<&'static str> {
        match self {
            Shape::Layer => None,
            Shape::Toplevel => Some(
                "no layer shell: the widget sits among windows rather than behind them, \
                 and cannot place itself — the compositor decides where it goes",
            ),
        }
    }
}

#[derive(Default)]
pub struct State {
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    pub wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub output: Option<wl_output::WlOutput>,
    /// Every global the compositor advertised, for the report.
    pub globals: Vec<(String, u32)>,
    /// Set when a configure has been acknowledged and the surface may be
    /// given a buffer.
    pub configured: bool,
    /// The size the compositor decided on. Zero means "you choose", which is
    /// what a layer surface says when it was given an explicit size.
    pub size: (u32, u32),
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
        state.globals.push((interface.clone(), version));
        match interface.as_str() {
            "wl_compositor" => {
                state.compositor = Some(registry.bind::<wl_compositor::WlCompositor, _, _>(
                    name,
                    4.min(version),
                    qh,
                    (),
                ));
            }
            "zwlr_layer_shell_v1" => {
                state.layer_shell = Some(
                    registry.bind::<zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(
                        name,
                        version.min(4),
                        qh,
                        (),
                    ),
                );
            }
            "xdg_wm_base" => {
                state.wm_base = Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(
                    name,
                    version.min(3),
                    qh,
                    (),
                ));
            }
            "wl_output" if state.output.is_none() => {
                state.output =
                    Some(registry.bind::<wl_output::WlOutput, _, _>(name, version.min(3), qh, ()));
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
                // attached — and therefore before wgpu may present. Driving it
                // here rather than letting wgpu drive it is the whole of
                // ARCHITECTURE.md §10.2.
                layer_surface.ack_configure(serial);
                if width != 0 && height != 0 {
                    state.size = (width, height);
                }
                state.configured = true;
            }
            zwlr_layer_surface_v1::Event::Closed => {
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
        if let xdg_toplevel::Event::Configure { width, height, .. } = event
            && width > 0
            && height > 0
        {
            state.size = (width as u32, height as u32);
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_output::WlOutput);
delegate_noop!(State: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

/// A configured desktop-layer surface, ready to be given to wgpu.
pub struct Surface {
    pub shape: Shape,
    pub surface: wl_surface::WlSurface,
    pub size: (u32, u32),
    /// Kept alive: dropping the role object destroys the role.
    _layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    _xdg: Option<(xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel)>,
}

impl Surface {
    /// The pointers wgpu needs. Valid as long as `self` and the connection are.
    pub fn raw_handles(
        &self,
        conn: &Connection,
    ) -> (
        raw_window_handle::RawDisplayHandle,
        raw_window_handle::RawWindowHandle,
    ) {
        let display = conn.backend().display_ptr() as *mut c_void;
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
}

/// Connects, binds, creates the surface and completes the configure handshake.
///
/// Returns with the surface configured and **no buffer attached** — which is
/// exactly the state wgpu needs to find it in.
pub fn connect(
    width: u32,
    height: u32,
) -> Result<
    (
        Connection,
        wayland_client::EventQueue<State>,
        State,
        Surface,
    ),
    String,
> {
    let conn = Connection::connect_to_env()
        .map_err(|e| format!("no Wayland display: {e} (is WAYLAND_DISPLAY set?)"))?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());

    let mut state = State::default();
    queue
        .roundtrip(&mut state)
        .map_err(|e| format!("registry roundtrip failed: {e}"))?;

    let compositor = state
        .compositor
        .clone()
        .ok_or("the compositor advertises no wl_compositor, which should be impossible")?;
    let surface = compositor.create_surface(&qh, ());

    // Forcing the fallback is how the degraded path gets tested at all: every
    // compositor that has a layer shell always has one, so the branch a GNOME
    // user takes would otherwise never run here.
    let forced_toplevel = std::env::var_os("SPIKE_FORCE_TOPLEVEL").is_some();
    let shell = if forced_toplevel {
        None
    } else {
        state.layer_shell.clone()
    };

    let (shape, layer_surface, xdg) = match shell {
        Some(shell) => {
            use zwlr_layer_shell_v1::Layer;
            use zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity};
            let layer_surface = shell.get_layer_surface(
                &surface,
                state.output.as_ref(),
                // `Bottom`, not `Background`: background is where a wallpaper
                // goes and it sits *under* the desktop's own surfaces.
                // `Bottom` is above the wallpaper and below every ordinary
                // window, which is the definition of a widget.
                Layer::Bottom,
                "uniwidged-spike".to_string(),
                &qh,
                (),
            );
            layer_surface.set_size(width, height);
            layer_surface.set_anchor(Anchor::Top | Anchor::Left);
            // Configurable so the stacking test can put the widget underneath
            // the window it is supposed to be underneath.
            let margin = |key: &str, fallback: i32| {
                std::env::var(key)
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(fallback)
            };
            layer_surface.set_margin(
                margin("SPIKE_MARGIN_TOP", 48),
                0,
                0,
                margin("SPIKE_MARGIN_LEFT", 48),
            );
            // A widget never takes the keyboard. This is the Wayland spelling
            // of `WS_EX_NOACTIVATE`.
            layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
            (Shape::Layer, Some(layer_surface), None)
        }
        None => {
            let wm_base = state
                .wm_base
                .clone()
                .ok_or("no layer shell and no xdg_wm_base — nothing to make a surface with")?;
            let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
            let toplevel = xdg_surface.get_toplevel(&qh, ());
            toplevel.set_title("uniwidged spike".into());
            toplevel.set_app_id("dev.uniwidged.spike".into());
            state.size = (width, height);
            (Shape::Toplevel, None, Some((xdg_surface, toplevel)))
        }
    };

    // Commit with no buffer: this is what asks for the configure.
    surface.commit();
    queue
        .roundtrip(&mut state)
        .map_err(|e| format!("configure roundtrip failed: {e}"))?;
    if !state.configured {
        return Err("the compositor never sent a configure".into());
    }

    let size = if state.size == (0, 0) {
        (width, height)
    } else {
        state.size
    };

    Ok((
        conn,
        queue,
        state,
        Surface {
            shape,
            surface,
            size,
            _layer_surface: layer_surface,
            _xdg: xdg,
        },
    ))
}
