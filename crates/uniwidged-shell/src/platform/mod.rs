//! One module per window system, selected at compile time.
//!
//! Linux has two, chosen at *run* time: a session can be Wayland or X11, and a
//! Wayland session can still run this through XWayland. Ephemeris picks by
//! inspecting the environment and says which it chose in the log, and so does
//! this.
//!
//! Every module implements the same shape — create a desktop-layer surface,
//! report [`crate::Capabilities`], pump events, hand out a
//! `raw_window_handle::RawWindowHandle` — and none of them is visible outside
//! this crate.

pub mod headless;

#[cfg(target_os = "windows")]
pub mod win32;

#[cfg(target_os = "macos")]
pub mod appkit;

#[cfg(all(unix, not(target_os = "macos")))]
pub mod wayland;

#[cfg(all(unix, not(target_os = "macos")))]
pub mod x11;
