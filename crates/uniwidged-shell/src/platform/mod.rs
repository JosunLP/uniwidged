//! One module per window system, selected at compile time.
//!
//! Linux has two, chosen at *run* time: a session can be Wayland or X11, and a
//! Wayland session can still run this through XWayland. Ephemeris picks by
//! inspecting the environment and says which it chose in the log, and so does
//! this.
//!
//! Every module implements the same shape — create a desktop-layer surface,
//! report [`crate::Capabilities`], pump events, hand out a
//! `raw_window_handle::RawWindowHandle` — and [`desktop`] gives that shape one
//! set of names, whichever platform this is.

pub mod headless;

#[cfg(target_os = "windows")]
pub mod win32;

#[cfg(target_os = "macos")]
pub mod appkit;

#[cfg(all(unix, not(target_os = "macos")))]
pub mod linux;

#[cfg(all(unix, not(target_os = "macos")))]
pub mod wayland;

#[cfg(all(unix, not(target_os = "macos")))]
pub mod x11;

/// Whichever of the above this build runs on, under one set of names.
///
/// The desktop loop in `uniwidged-cli` is written against these and nothing
/// else: a [`desktop::DesktopWindow`] per widget, [`desktop::outputs`] before
/// the first one exists, and [`desktop::wait_any`] to block on all of them and
/// the wake pipe at once. Each platform module provides exactly that shape —
/// the same methods, the same meaning — so the loop has no branch for a
/// platform anywhere in it.
pub mod desktop {
    #[cfg(all(unix, not(target_os = "macos")))]
    pub use super::linux::{DesktopError, DesktopWindow, outputs, wait_any};

    #[cfg(target_os = "windows")]
    pub use super::win32::{DesktopError, DesktopWindow, outputs, wait_any};

    #[cfg(target_os = "macos")]
    pub use super::appkit::{DesktopError, DesktopWindow, outputs, wait_any};
}
