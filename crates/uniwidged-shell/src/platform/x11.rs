//! The widget's window on X11.
//!
//! Adapted from Ephemeris `src/unix/linux/window.rs`.
//!
//! Every property has a standard here, which is why X11 was the easier of the
//! two for Ephemeris:
//!
//! | | |
//! |---|---|
//! | below every window, above the desktop | `_NET_WM_STATE_BELOW` |
//! | no taskbar or switcher entry | `_NET_WM_STATE_SKIP_TASKBAR`, `_SKIP_PAGER` |
//! | on every virtual desktop | `_NET_WM_STATE_STICKY` |
//! | never takes focus | `WM_HINTS.input = False`, no `WM_TAKE_FOCUS` |
//! | no frame | `_MOTIF_WM_HINTS` with the decorations bit cleared |
//! | per-pixel alpha | a 32-bit `TrueColor` visual and its own colormap |
//!
//! `_NET_WM_WINDOW_TYPE_DESKTOP` was the other candidate for the first row and
//! is wrong: it means the window *is* the desktop, and window managers that
//! honour it stack the widget beneath the desktop icons and stop delivering
//! clicks. `BELOW` on an ordinary window says what is actually wanted.
//!
//! ## The transparency question
//!
//! Ephemeris got a transparent X11 window by drawing into that 32-bit visual
//! with Cairo, and Cairo does not care what the GPU thinks. A Vulkan swapchain
//! does. [`AlphaPath`] names the three ways out, tried in order; which one a
//! session ended up on is reported rather than assumed, because the symptom of
//! getting it wrong is visual and the cause is not.
//!
//! **A compositor is required either way.** Without one the X server ignores a
//! window's alpha channel entirely and draws it opaque, whatever the visual
//! says — so [`X11Window::has_compositor`] is checked at start-up and said out
//! loud, in the same breath as everything else that is missing.
//!
//! ## Why libxcb rather than x11rb's own connection
//!
//! wgpu creates a surface from an `xcb_connection_t*`, and x11rb's pure-Rust
//! connection has no pointer to give it. So this uses `XCBConnection`, which
//! wraps the C library — the same one Mesa is already talking to, which is
//! also what makes them agree about the connection's state.

use std::ptr::NonNull;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    self, AtomEnum, ColormapAlloc, ConnectionExt, CreateWindowAux, EventMask, PropMode,
    VisualClass, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::xcb_ffi::XCBConnection;

use crate::{Capabilities, Placement};
use uniwidged_core::geom::PhysicalRect;

/// Which of the three transparency paths this session ended up on.
///
/// See `ARCHITECTURE.md` §10.1. They are tried in this order, and the first
/// that works is the one used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaPath {
    /// A Vulkan swapchain that accepts pre-multiplied alpha. Best case: the
    /// GPU rasterises and the GPU presents.
    VulkanComposite,
    /// EGL with an ARGB visual. EGL on X11 does honour the visual's alpha
    /// channel where Vulkan's WSI will not.
    EglArgb,
    /// Vello renders to a texture as it does everywhere else; the pixels are
    /// read back and put into the window with `PutImage`. Rasterisation is
    /// untouched — the same scene, the same GPU, the same output — so the
    /// golden images still apply. Only *presentation* falls back, which is the
    /// thing X11 is actually bad at.
    ///
    /// No XRender is involved, which the plan assumed there would be: the
    /// window already has a 32-bit visual, so the compositor blends its
    /// contents exactly as it blends a swapchain's. Putting premultiplied BGRA
    /// straight into it is the whole of the fallback.
    Readback,
}

impl AlphaPath {
    pub fn capabilities(self) -> Capabilities {
        Capabilities {
            gpu_swapchain: !matches!(self, AlphaPath::Readback),
            ..Capabilities::FULL
        }
    }

    /// What to put in the start-up log.
    pub fn describe(self) -> &'static str {
        match self {
            AlphaPath::VulkanComposite => "GPU swapchain with pre-multiplied alpha",
            AlphaPath::EglArgb => "EGL with an ARGB visual",
            AlphaPath::Readback => "readback presentation, blended by the compositor",
        }
    }
}

/// Why a window could not be created.
#[derive(Debug)]
pub enum X11Error {
    Connect(String),
    /// No 32-bit `TrueColor` visual. Without one there is no alpha channel to
    /// have, so the widget would be an opaque rectangle whatever else works.
    NoArgbVisual,
    Protocol(String),
}

impl std::fmt::Display for X11Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            X11Error::Connect(why) => write!(f, "cannot reach the X server: {why}"),
            X11Error::NoArgbVisual => write!(
                f,
                "this display offers no 32-bit TrueColor visual, so a widget \
                 cannot be transparent on it at all"
            ),
            X11Error::Protocol(why) => write!(f, "X11: {why}"),
        }
    }
}

impl std::error::Error for X11Error {}

fn protocol<E: std::fmt::Display>(error: E) -> X11Error {
    X11Error::Protocol(error.to_string())
}

/// The atoms the widget needs, interned once.
///
/// Gathered in a struct rather than looked up where they are used: each lookup
/// is a round trip to the server, and several of them happen while the window
/// is being set up.
#[allow(non_snake_case)]
struct Atoms {
    net_wm_state: xproto::Atom,
    net_wm_state_below: xproto::Atom,
    net_wm_state_skip_taskbar: xproto::Atom,
    net_wm_state_skip_pager: xproto::Atom,
    net_wm_state_sticky: xproto::Atom,
    net_wm_window_type: xproto::Atom,
    net_wm_window_type_utility: xproto::Atom,
    net_wm_name: xproto::Atom,
    net_wm_desktop: xproto::Atom,
    motif_wm_hints: xproto::Atom,
    wm_protocols: xproto::Atom,
    wm_delete_window: xproto::Atom,
    utf8_string: xproto::Atom,
}

impl Atoms {
    fn intern(conn: &XCBConnection) -> Result<Self, X11Error> {
        // Every request is sent before any reply is awaited, so the whole set
        // costs one round trip rather than thirteen.
        let names: [&[u8]; 13] = [
            b"_NET_WM_STATE",
            b"_NET_WM_STATE_BELOW",
            b"_NET_WM_STATE_SKIP_TASKBAR",
            b"_NET_WM_STATE_SKIP_PAGER",
            b"_NET_WM_STATE_STICKY",
            b"_NET_WM_WINDOW_TYPE",
            b"_NET_WM_WINDOW_TYPE_UTILITY",
            b"_NET_WM_NAME",
            b"_NET_WM_DESKTOP",
            b"_MOTIF_WM_HINTS",
            b"WM_PROTOCOLS",
            b"WM_DELETE_WINDOW",
            b"UTF8_STRING",
        ];
        let cookies: Vec<_> = names
            .iter()
            .map(|name| conn.intern_atom(false, name))
            .collect::<Result<_, _>>()
            .map_err(protocol)?;
        let mut atoms = Vec::with_capacity(names.len());
        for cookie in cookies {
            atoms.push(cookie.reply().map_err(protocol)?.atom);
        }
        Ok(Self {
            net_wm_state: atoms[0],
            net_wm_state_below: atoms[1],
            net_wm_state_skip_taskbar: atoms[2],
            net_wm_state_skip_pager: atoms[3],
            net_wm_state_sticky: atoms[4],
            net_wm_window_type: atoms[5],
            net_wm_window_type_utility: atoms[6],
            net_wm_name: atoms[7],
            net_wm_desktop: atoms[8],
            motif_wm_hints: atoms[9],
            wm_protocols: atoms[10],
            wm_delete_window: atoms[11],
            utf8_string: atoms[12],
        })
    }
}

/// What kind of window this is.
///
/// A host needs both: the widgets themselves, and the occasional ordinary
/// window for settings or a permission prompt — which must behave like a
/// window, take focus, and appear in the switcher. Making it a parameter
/// rather than a second code path keeps the two from drifting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowKind {
    /// Below every ordinary window, never focused, no switcher entry.
    #[default]
    DesktopWidget,
    /// An ordinary application window.
    Ordinary,
}

/// A window on X11.
pub struct X11Window {
    conn: XCBConnection,
    screen: usize,
    window: xproto::Window,
    visual: xproto::Visualid,
    atoms: Atoms,
    placement: Placement,
    has_compositor: bool,
}

impl X11Window {
    /// Creates a desktop-layer window, sets every property, and maps it.
    pub fn open(rect: PhysicalRect, title: &str) -> Result<Self, X11Error> {
        Self::open_kind(rect, title, WindowKind::DesktopWidget)
    }

    /// As [`X11Window::open`], for a window of any kind.
    pub fn open_kind(rect: PhysicalRect, title: &str, kind: WindowKind) -> Result<Self, X11Error> {
        let (conn, screen_num) =
            XCBConnection::connect(None).map_err(|e| X11Error::Connect(e.to_string()))?;
        let setup = conn.setup();
        let screen = setup.roots[screen_num].clone();

        let (depth, visual) = argb_visual(&screen).ok_or(X11Error::NoArgbVisual)?;
        let atoms = Atoms::intern(&conn)?;

        // A window whose depth differs from its parent's needs its own
        // colormap *and* an explicit border pixel. Without the border pixel
        // the server answers `BadMatch`, which is a confusing way to be told
        // "you forgot a field".
        let colormap = conn.generate_id().map_err(protocol)?;
        conn.create_colormap(ColormapAlloc::NONE, colormap, screen.root, visual)
            .map_err(protocol)?;

        let window = conn.generate_id().map_err(protocol)?;
        let aux = CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .colormap(colormap)
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::STRUCTURE_NOTIFY
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION
                    | EventMask::LEAVE_WINDOW,
            );
        conn.create_window(
            depth,
            window,
            screen.root,
            rect.x as i16,
            rect.y as i16,
            rect.width as u16,
            rect.height as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &aux,
        )
        .map_err(protocol)?;

        let mut this = Self {
            conn,
            screen: screen_num,
            window,
            visual,
            atoms,
            placement: Placement {
                rect,
                output: None,
                // X11 has no per-window scale factor; a session picks one and
                // Xft.dpi is where a desktop records it. Read in M7, when
                // there is a configuration to read it into.
                scale: 1.0,
            },
            has_compositor: false,
        };
        this.set_properties(title, kind)?;
        this.has_compositor = this.detect_compositor()?;

        this.conn.map_window(this.window).map_err(protocol)?;
        // Position again after mapping: a window manager is entitled to place a
        // window where it likes, and several do. Ephemeris learned the same
        // thing.
        this.conn
            .configure_window(
                this.window,
                &xproto::ConfigureWindowAux::new()
                    .x(rect.x)
                    .y(rect.y)
                    .width(rect.width)
                    .height(rect.height),
            )
            .map_err(protocol)?;
        this.conn.flush().map_err(protocol)?;
        Ok(this)
    }

    fn set_properties(&self, title: &str, kind: WindowKind) -> Result<(), X11Error> {
        let conn = &self.conn;
        let w = self.window;
        let a = &self.atoms;

        if kind == WindowKind::DesktopWidget {
            // The four states that make it a widget rather than a window. Set
            // as a property before mapping, which is how the specification says
            // an unmapped window asks for them — a client message is for
            // changing them afterwards.
            conn.change_property32(
                PropMode::REPLACE,
                w,
                a.net_wm_state,
                AtomEnum::ATOM,
                &[
                    a.net_wm_state_below,
                    a.net_wm_state_skip_taskbar,
                    a.net_wm_state_skip_pager,
                    a.net_wm_state_sticky,
                ],
            )
            .map_err(protocol)?;

            // `UTILITY`, not `DESKTOP` — see the module note on why `DESKTOP`
            // is the wrong answer. `UTILITY` tells a window manager this is
            // auxiliary without telling it the window *is* the desktop.
            conn.change_property32(
                PropMode::REPLACE,
                w,
                a.net_wm_window_type,
                AtomEnum::ATOM,
                &[a.net_wm_window_type_utility],
            )
            .map_err(protocol)?;

            // On every virtual desktop. `STICKY` above asks the window manager;
            // this says the same thing in the older way, and between them every
            // window manager understands one.
            conn.change_property32(
                PropMode::REPLACE,
                w,
                a.net_wm_desktop,
                AtomEnum::CARDINAL,
                &[0xFFFF_FFFFu32],
            )
            .map_err(protocol)?;
        }

        conn.change_property8(
            PropMode::REPLACE,
            w,
            a.net_wm_name,
            a.utf8_string,
            title.as_bytes(),
        )
        .map_err(protocol)?;
        conn.change_property8(
            PropMode::REPLACE,
            w,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            title.as_bytes(),
        )
        .map_err(protocol)?;
        // `WM_CLASS` is two NUL-terminated strings, instance then class. A
        // window manager keys per-application rules off it.
        conn.change_property8(
            PropMode::REPLACE,
            w,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"uniwidged\0uniwidged\0",
        )
        .map_err(protocol)?;

        if kind == WindowKind::Ordinary {
            // An ordinary window keeps its frame, its switcher entry and its
            // focus. Everything below this point is shared.
            conn.flush().map_err(protocol)?;
            return Ok(());
        }

        // No frame. `_MOTIF_WM_HINTS` is five words; the first is a flags
        // field where bit 1 means "the decorations field is meaningful", and
        // the third is that field. Zero decorations, no titlebar, no border.
        conn.change_property32(
            PropMode::REPLACE,
            w,
            a.motif_wm_hints,
            a.motif_wm_hints,
            &[2u32, 0, 0, 0, 0],
        )
        .map_err(protocol)?;

        // Never takes focus. `WM_HINTS` is nine words; bit 0 of the flags says
        // the input field is meaningful, and the input field is `False`. This
        // is X11's spelling of `WS_EX_NOACTIVATE`, and it is what separates a
        // gadget from a window in the way.
        //
        // `WM_PROTOCOLS` deliberately does *not* list `WM_TAKE_FOCUS`: a
        // window that declines input but offers to take focus is asking for
        // both at once, and window managers resolve that inconsistently.
        conn.change_property32(
            PropMode::REPLACE,
            w,
            AtomEnum::WM_HINTS,
            AtomEnum::WM_HINTS,
            &[1u32, 0, 0, 0, 0, 0, 0, 0, 0],
        )
        .map_err(protocol)?;
        conn.change_property32(
            PropMode::REPLACE,
            w,
            a.wm_protocols,
            AtomEnum::ATOM,
            &[a.wm_delete_window],
        )
        .map_err(protocol)?;
        Ok(())
    }

    /// Is a compositing manager running?
    ///
    /// Asked as a selection owner, because that is what it is —
    /// `_NET_WM_CM_S<n>` is a selection a compositor acquires, not a property
    /// on the root window. Looking for it as a property finds nothing on a
    /// perfectly composited desktop, which is a mistake worth only making
    /// once.
    fn detect_compositor(&self) -> Result<bool, X11Error> {
        let name = format!("_NET_WM_CM_S{}", self.screen);
        let atom = self
            .conn
            .intern_atom(false, name.as_bytes())
            .map_err(protocol)?
            .reply()
            .map_err(protocol)?
            .atom;
        let owner = self
            .conn
            .get_selection_owner(atom)
            .map_err(protocol)?
            .reply()
            .map_err(protocol)?
            .owner;
        Ok(owner != x11rb::NONE)
    }

    /// Whether a compositing manager is running.
    ///
    /// Without one the X server ignores the window's alpha channel and draws
    /// it opaque, whatever visual it has — so this is the difference between a
    /// widget and a rectangle, and it is reported rather than assumed.
    pub fn has_compositor(&self) -> bool {
        self.has_compositor
    }

    pub fn window_id(&self) -> u32 {
        self.window
    }

    pub fn visual_id(&self) -> u32 {
        self.visual
    }

    pub fn placement(&self) -> &Placement {
        &self.placement
    }

    /// What this session can actually deliver.
    pub fn capabilities(&self, path: AlphaPath) -> Capabilities {
        Capabilities {
            per_pixel_alpha: self.has_compositor,
            ..path.capabilities()
        }
    }

    /// The handles wgpu needs. Valid as long as `self` is.
    pub fn raw_handles(
        &self,
    ) -> (
        raw_window_handle::RawDisplayHandle,
        raw_window_handle::RawWindowHandle,
    ) {
        let mut display = raw_window_handle::XcbDisplayHandle::new(
            NonNull::new(self.conn.get_raw_xcb_connection()),
            self.screen as i32,
        );
        display.screen = self.screen as i32;
        let mut window = raw_window_handle::XcbWindowHandle::new(
            std::num::NonZeroU32::new(self.window).expect("an X11 window id is never zero"),
        );
        // wgpu picks the surface's format from the visual, so a window created
        // on a 32-bit visual and described with a different one would present
        // opaque for no visible reason.
        window.visual_id = std::num::NonZeroU32::new(self.visual);
        (
            raw_window_handle::RawDisplayHandle::Xcb(display),
            raw_window_handle::RawWindowHandle::Xcb(window),
        )
    }

    pub fn flush(&self) -> Result<(), X11Error> {
        self.conn.flush().map_err(protocol)
    }

    /// Puts finished pixels straight into the window.
    ///
    /// This is [`AlphaPath::Readback`]: Vello rasterised to a texture as it
    /// does everywhere else, the pixels were read back, and they are handed to
    /// the X server here. The compositor does the blending, exactly as it does
    /// for a GPU swapchain — which is why this fallback gives up the GPU
    /// *presentation* path and nothing else. The scene, the rasteriser and the
    /// output are unchanged, so the golden images still apply.
    ///
    /// `rgba` is straight-alpha RGBA8, top row first — what
    /// `uniwidged_render::Image` holds. The two conversions X11 needs are done
    /// here rather than by the caller:
    ///
    /// * **Premultiplied**, because that is what a compositor blends.
    /// * **BGRA byte order**, because a little-endian server's 32-bit
    ///   `TrueColor` visual stores blue first. Getting this wrong swaps red
    ///   and blue, which looks like a theme bug rather than a byte-order one.
    pub fn present_rgba(&self, width: u32, height: u32, rgba: &[u8]) -> Result<(), X11Error> {
        let expected = (width as usize) * (height as usize) * 4;
        if rgba.len() != expected {
            return Err(X11Error::Protocol(format!(
                "expected {expected} bytes for {width}x{height}, got {}",
                rgba.len()
            )));
        }
        let mut bgra = Vec::with_capacity(expected);
        for pixel in rgba.as_chunks::<4>().0 {
            let alpha = pixel[3] as u32;
            let premultiply = |c: u8| ((c as u32 * alpha + 127) / 255) as u8;
            bgra.push(premultiply(pixel[2]));
            bgra.push(premultiply(pixel[1]));
            bgra.push(premultiply(pixel[0]));
            bgra.push(pixel[3]);
        }

        let gc = self.conn.generate_id().map_err(protocol)?;
        self.conn
            .create_gc(gc, self.window, &xproto::CreateGCAux::new())
            .map_err(protocol)?;
        // One request. A larger surface would have to be split into bands to
        // stay under the server's maximum request size; at widget sizes it
        // never comes close, and splitting untested code is worse than not
        // having it.
        self.conn
            .put_image(
                xproto::ImageFormat::Z_PIXMAP,
                self.window,
                gc,
                width as u16,
                height as u16,
                0,
                0,
                0,
                32,
                &bgra,
            )
            .map_err(protocol)?;
        self.conn.free_gc(gc).map_err(protocol)?;
        self.conn.flush().map_err(protocol)?;
        Ok(())
    }
}

/// The first 32-bit `TrueColor` visual on this screen, with its depth.
fn argb_visual(screen: &xproto::Screen) -> Option<(u8, xproto::Visualid)> {
    screen
        .allowed_depths
        .iter()
        .find(|depth| depth.depth == 32)
        .and_then(|depth| {
            depth
                .visuals
                .iter()
                .find(|visual| visual.class == VisualClass::TRUE_COLOR)
                .map(|visual| (depth.depth, visual.visual_id))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_x11_path_keeps_per_pixel_alpha() {
        // The fallbacks give up GPU rasterisation, never transparency. A
        // widget on an opaque rectangle is not a widget.
        for path in [
            AlphaPath::VulkanComposite,
            AlphaPath::EglArgb,
            AlphaPath::Readback,
        ] {
            assert!(path.capabilities().per_pixel_alpha, "{path:?}");
            assert!(path.capabilities().below_windows, "{path:?}");
            assert!(!path.describe().is_empty());
        }
        assert!(!AlphaPath::Readback.capabilities().gpu_swapchain);
        assert!(AlphaPath::EglArgb.capabilities().gpu_swapchain);
    }
}
