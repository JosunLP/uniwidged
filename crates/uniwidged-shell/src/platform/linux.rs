//! Wayland or X11, chosen when the desktop starts.
//!
//! Linux is the one platform with two window systems in use at once, and a
//! binary has to work on both — a user does not rebuild uniwidged when they log
//! into a different session. So the choice is made at *run* time, here, and
//! everything above this file is written against one type.
//!
//! ## How the session is picked
//!
//! `WAYLAND_DISPLAY` first, because a Wayland session that also runs XWayland
//! sets both — and going through XWayland when a native protocol is available
//! means giving up the layer shell, which is the whole definition of a desktop
//! widget. If the Wayland connection fails for any reason, X11 is tried anyway
//! rather than the desktop refusing to start: a widget on the wrong shell is
//! better than no widget, and [`DesktopWindow::describe`] says which one
//! happened.
//!
//! ## Why an enum and not a trait
//!
//! Two implementations, both in this crate, chosen once at start-up and never
//! mixed. A trait would add a vtable, a lifetime and an object-safety
//! constraint on every method — `raw_handles` returns two raw handles by value
//! — to abstract over a set that is closed and will stay closed. The match
//! arms below are the whole cost, and they are the place a reader can see the
//! two platforms side by side, which is worth more here than the indirection
//! would be.

use std::path::Path;
use std::time::Duration;

use uniwidged_core::geom::PhysicalRect;

use super::wayland::{self, Prefer, WaylandWindow};
use super::x11::{self, X11Window};
use crate::{Capabilities, Input, Output};

/// Which window system a desktop ended up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    Wayland,
    X11,
}

#[derive(Debug)]
pub enum DesktopError {
    /// Neither window system could be reached, with what each said.
    NoSession {
        wayland: Option<String>,
        x11: String,
    },
    Wayland(wayland::WaylandError),
    X11(x11::X11Error),
}

impl std::fmt::Display for DesktopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DesktopError::NoSession { wayland, x11 } => {
                write!(f, "no desktop session to put a widget on: ")?;
                if let Some(wayland) = wayland {
                    write!(f, "{wayland}; ")?;
                }
                write!(f, "{x11}")
            }
            DesktopError::Wayland(e) => write!(f, "{e}"),
            DesktopError::X11(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DesktopError {}

/// What the environment says the session is.
///
/// A guess, and only used for ordering the attempts — what actually happened
/// is [`DesktopWindow::session`], which is the one worth reporting.
pub fn session() -> Session {
    match std::env::var_os("WAYLAND_DISPLAY") {
        Some(display) if !display.is_empty() => Session::Wayland,
        _ => Session::X11,
    }
}

/// The monitors this session has, before any window exists.
///
/// Placement is decided before a window is created — a window's initial
/// geometry *is* its placement — so this is asked first, and answers from
/// whichever session is there. An empty list means "cannot say", which
/// `uniwidged_host::placement` treats as "use the stored coordinates as they
/// stand".
pub fn outputs() -> Vec<Output> {
    if session() == Session::Wayland {
        // Wayland reports outputs through a connection, and a connection here
        // means a surface. Rather than create one to throw away, the desktop
        // asks the first window it opens — see `DesktopWindow::outputs`. What
        // is returned here is X11's answer, or nothing.
        return Vec::new();
    }
    x11::outputs(None).unwrap_or_default()
}

/// One widget's surface, on whichever window system this session runs.
pub enum DesktopWindow {
    Wayland(Box<WaylandWindow>),
    X11(Box<X11Window>),
}

impl DesktopWindow {
    /// Opens a desktop-layer surface, preferring the native session.
    pub fn open(rect: PhysicalRect, title: &str) -> Result<Self, DesktopError> {
        let mut wayland_said = None;
        if session() == Session::Wayland {
            match WaylandWindow::open_on(None, rect, title, Prefer::LayerShell) {
                Ok(window) => return Ok(DesktopWindow::Wayland(Box::new(window))),
                // Falls through to X11 rather than failing: a Wayland session
                // running XWayland can still show a widget, and saying so is
                // better than refusing to start.
                Err(e) => wayland_said = Some(e.to_string()),
            }
        }
        match X11Window::open(rect, title) {
            Ok(window) => Ok(DesktopWindow::X11(Box::new(window))),
            Err(x11) => Err(DesktopError::NoSession {
                wayland: wayland_said,
                x11: x11.to_string(),
            }),
        }
    }

    /// As [`DesktopWindow::open`], on a named Wayland socket or X11 display.
    ///
    /// For a test that starts its own compositor, and for running two desktops
    /// side by side.
    pub fn open_on(
        session: Session,
        display: Option<&Path>,
        rect: PhysicalRect,
        title: &str,
    ) -> Result<Self, DesktopError> {
        match session {
            Session::Wayland => WaylandWindow::open_on(display, rect, title, Prefer::LayerShell)
                .map(|w| DesktopWindow::Wayland(Box::new(w)))
                .map_err(DesktopError::Wayland),
            Session::X11 => X11Window::open_on(
                display.map(|p| p.to_string_lossy().into_owned()).as_deref(),
                rect,
                title,
                x11::WindowKind::DesktopWidget,
            )
            .map(|w| DesktopWindow::X11(Box::new(w)))
            .map_err(DesktopError::X11),
        }
    }

    pub fn session(&self) -> Session {
        match self {
            DesktopWindow::Wayland(_) => Session::Wayland,
            DesktopWindow::X11(_) => Session::X11,
        }
    }

    /// One line for the start-up log, naming the shell and anything lost.
    pub fn describe(&self) -> String {
        let (what, capabilities) = match self {
            DesktopWindow::Wayland(w) => (
                match w.shape() {
                    wayland::Shape::Layer => "Wayland (wlr-layer-shell, bottom layer)",
                    wayland::Shape::Toplevel => "Wayland (xdg_toplevel — no layer shell here)",
                },
                w.capabilities(),
            ),
            DesktopWindow::X11(w) => ("X11", w.capabilities(x11::AlphaPath::VulkanComposite)),
        };
        match capabilities.degradation() {
            Some(lost) => format!("{what}; degraded: {lost}"),
            None => what.to_string(),
        }
    }

    pub fn capabilities(&self) -> Capabilities {
        match self {
            DesktopWindow::Wayland(w) => w.capabilities(),
            DesktopWindow::X11(w) => w.capabilities(x11::AlphaPath::VulkanComposite),
        }
    }

    pub fn has_compositor(&self) -> bool {
        match self {
            DesktopWindow::Wayland(w) => w.has_compositor(),
            DesktopWindow::X11(w) => w.has_compositor(),
        }
    }

    pub fn window_id(&self) -> u32 {
        match self {
            DesktopWindow::Wayland(w) => w.window_id(),
            DesktopWindow::X11(w) => w.window_id(),
        }
    }

    pub fn raw_handles(
        &self,
    ) -> (
        raw_window_handle::RawDisplayHandle,
        raw_window_handle::RawWindowHandle,
    ) {
        match self {
            DesktopWindow::Wayland(w) => w.raw_handles(),
            DesktopWindow::X11(w) => w.raw_handles(),
        }
    }

    pub fn geometry(&self) -> PhysicalRect {
        match self {
            DesktopWindow::Wayland(w) => w.geometry(),
            DesktopWindow::X11(w) => w.geometry(),
        }
    }

    pub fn set_geometry(&mut self, rect: PhysicalRect) -> Result<(), DesktopError> {
        match self {
            DesktopWindow::Wayland(w) => w.set_geometry(rect).map_err(DesktopError::Wayland),
            DesktopWindow::X11(w) => w.set_geometry(rect).map_err(DesktopError::X11),
        }
    }

    pub fn desktop_bounds(&self) -> PhysicalRect {
        match self {
            DesktopWindow::Wayland(w) => w.desktop_bounds(),
            DesktopWindow::X11(w) => w.desktop_bounds(),
        }
    }

    pub fn outputs(&self) -> Vec<Output> {
        match self {
            DesktopWindow::Wayland(w) => w.outputs(),
            DesktopWindow::X11(w) => w.outputs().unwrap_or_default(),
        }
    }

    pub fn grab_pointer(&self) -> Result<(), DesktopError> {
        match self {
            // Wayland grabs implicitly for the duration of a press, so there
            // is nothing to ask for.
            DesktopWindow::Wayland(_) => Ok(()),
            DesktopWindow::X11(w) => w.grab_pointer().map_err(DesktopError::X11),
        }
    }

    pub fn release_pointer(&self) -> Result<(), DesktopError> {
        match self {
            DesktopWindow::Wayland(_) => Ok(()),
            DesktopWindow::X11(w) => w.release_pointer().map_err(DesktopError::X11),
        }
    }

    pub fn wait(&mut self, timeout: Option<Duration>) -> Result<(), DesktopError> {
        self.wait_with(timeout, None)
    }

    /// As [`DesktopWindow::wait`], also waking when a background task
    /// finishes — see `crate::wake` for why that needs a descriptor rather
    /// than a flag.
    pub fn wait_with(
        &mut self,
        timeout: Option<Duration>,
        wake: Option<&crate::WakePipe>,
    ) -> Result<(), DesktopError> {
        match self {
            DesktopWindow::Wayland(w) => w.wait_with(timeout, wake).map_err(DesktopError::Wayland),
            DesktopWindow::X11(w) => w.wait_with(timeout, wake).map_err(DesktopError::X11),
        }
    }

    pub fn drain_events(&mut self) -> Result<Vec<Input>, DesktopError> {
        match self {
            DesktopWindow::Wayland(w) => w.drain_events().map_err(DesktopError::Wayland),
            DesktopWindow::X11(w) => w.drain_events().map_err(DesktopError::X11),
        }
    }

    /// The descriptor this window's events arrive on.
    pub fn poll_fd(&self) -> std::os::fd::RawFd {
        match self {
            DesktopWindow::Wayland(w) => w.poll_fd(),
            DesktopWindow::X11(w) => w.poll_fd(),
        }
    }

    /// Sends everything this client has buffered.
    pub fn flush(&self) -> Result<(), DesktopError> {
        match self {
            DesktopWindow::Wayland(w) => w.flush().map_err(DesktopError::Wayland),
            DesktopWindow::X11(w) => w.flush().map_err(DesktopError::X11),
        }
    }

    /// Whether anything is already waiting to become an [`Input`].
    pub fn has_queued(&mut self) -> Result<bool, DesktopError> {
        match self {
            DesktopWindow::Wayland(w) => w.has_queued().map_err(DesktopError::Wayland),
            DesktopWindow::X11(w) => w.has_queued().map_err(DesktopError::X11),
        }
    }

    /// A size the *compositor* chose, if it has chosen a new one.
    ///
    /// Only Wayland does this: a layer surface's size is the compositor's
    /// decision and it may be smaller than asked for. X11 gives a client what
    /// it configured, so there is never anything to report.
    pub fn take_resize(&mut self) -> Option<(u32, u32)> {
        match self {
            DesktopWindow::Wayland(w) => w.take_resize(),
            DesktopWindow::X11(_) => None,
        }
    }

    /// Whatever this session's readback path needs, for the X11 mitigation.
    pub fn present_rgba(&self, width: u32, height: u32, rgba: &[u8]) -> Result<(), DesktopError> {
        match self {
            // Wayland has no readback path: `VK_COMPOSITE_ALPHA_PRE_MULTIPLIED`
            // is advertised there, which is why X11 is the platform with the
            // fallback and this one is not.
            DesktopWindow::Wayland(_) => Ok(()),
            DesktopWindow::X11(w) => w
                .present_rgba(width, height, rgba)
                .map_err(DesktopError::X11),
        }
    }
}

/// Blocks until *any* of these windows has something to say, or the timeout
/// elapses, or a background task finishes.
///
/// ## Why this exists
///
/// Every window on Linux carries its own connection — `XCBConnection::connect`
/// per X11 window, `wl_display` per Wayland one — because a window is opened
/// by whoever wants one and there is no session object above them. A desktop
/// runs one loop over N widgets, so it has N descriptors, and
/// [`DesktopWindow::wait_with`] waits on exactly one.
///
/// Waiting on the first window is enough only when the first window is the one
/// the user touched. It is not, and the consequence was precise: on an idle
/// desktop `next_timeout` is `None` — the property `ARCHITECTURE.md` §5 is
/// built around — so the loop blocked with no timer, and a click on the second
/// widget sat unread on the second connection until something *else* woke the
/// process. Two widgets, and one of them stopped responding to the pointer.
///
/// So: one `poll` over every connection and the wake pipe together.
///
/// ## The order, which is the whole of the correctness
///
/// 1. **Ask every window whether it already has something.** An event that
///    arrived between the last drain and this call is in a queue, not on a
///    socket, and no amount of polling will report it. Returning early here is
///    what stops the loop sleeping through work it already has.
/// 2. **Flush.** A request still in this client's buffer is one the server has
///    not seen, and waiting for a reply to it would wait forever.
/// 3. **Poll every descriptor at once**, plus the wake pipe.
/// 4. **Let every window read what it has**, with a zero timeout. Which
///    descriptor `poll` reported is deliberately not matched back to a window:
///    a zero-timeout wait costs one syscall on a window with nothing to say,
///    and a mismatch would cost an event.
///
/// An empty slice is a desktop whose widgets have all been removed. It waits
/// on the wake pipe and the timeout alone rather than returning immediately,
/// which is what keeps such a desktop idle instead of spinning.
pub fn wait_any(
    windows: &mut [DesktopWindow],
    timeout: Option<Duration>,
    wake: Option<&crate::WakePipe>,
) -> Result<(), DesktopError> {
    use std::os::fd::AsRawFd;

    for window in windows.iter_mut() {
        if window.has_queued()? {
            return Ok(());
        }
    }
    for window in windows.iter() {
        window.flush()?;
    }

    let millis = match timeout {
        Some(d) => d.as_millis().min(i32::MAX as u128) as i32,
        None => -1,
    };
    let mut fds: Vec<libc::pollfd> = windows
        .iter()
        .map(|window| libc::pollfd {
            fd: window.poll_fd(),
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    if let Some(wake) = wake {
        fds.push(libc::pollfd {
            fd: wake.as_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
    }
    // A desktop with no windows and no wake pipe has nothing to wait for and no
    // way to be told; sleeping the timeout is the only thing left that is not a
    // busy loop.
    if fds.is_empty() {
        if let Some(d) = timeout {
            std::thread::sleep(d);
        }
        return Ok(());
    }

    // SAFETY: descriptors borrowed for the duration of the call and not stored.
    let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, millis) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        // A signal is not a failure; the loop simply comes round again. This is
        // also how `SIGHUP` ends the block — see `uniwidged_cli::daemon`.
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(DesktopError::X11(super::x11::X11Error::Protocol(
                error.to_string(),
            )));
        }
        return Ok(());
    }
    // Emptied whether or not it is what woke us: a pipe left readable makes
    // every later poll return immediately, and an idle desktop becomes a busy
    // one.
    if let Some(wake) = wake {
        wake.drain();
    }
    for window in windows.iter_mut() {
        window.wait_with(Some(Duration::ZERO), None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_with_no_session_at_all_says_what_both_tried() {
        // The message a user gets on a machine with neither, which is a
        // container more often than anybody expects. Naming only one of the
        // two sends them to fix the wrong thing.
        let error = DesktopError::NoSession {
            wayland: Some("no Wayland display: nowhere".into()),
            x11: "cannot connect to X server".into(),
        };
        let said = error.to_string();
        assert!(said.contains("Wayland"), "{said}");
        assert!(said.contains("X server"), "{said}");
    }

    #[test]
    fn an_x11_only_machine_does_not_mention_wayland() {
        let error = DesktopError::NoSession {
            wayland: None,
            x11: "cannot connect to X server".into(),
        };
        assert!(!error.to_string().contains("Wayland"));
    }
}
