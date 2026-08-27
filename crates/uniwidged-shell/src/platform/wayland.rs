//! The widget's surface on Wayland.
//!
//! Adapted from Ephemeris `src/unix/linux/wayland/window.rs`.
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
//! because Wayland gives no client that power. Both go in the log at start-up.
//!
//! ## What is new here, relative to Ephemeris
//!
//! Ephemeris attached a Cairo image buffer in shared memory. uniwidged hands
//! the same `wl_surface` to wgpu as a `RawWindowHandle::Wayland` and lets Vello
//! render into a swapchain backed by a dma-buf. The surface role is irrelevant
//! to that — a layer surface is a `wl_surface` like any other — but the
//! *lifecycle* is not, and this is the delicate part:
//!
//! * A layer surface must `ack_configure` before its first commit, and the
//!   size it is given comes from the compositor. wgpu's surface API assumes it
//!   owns configure and present timing, so the sequence has to be driven from
//!   here with wgpu attached to it, not the other way round. See
//!   `ARCHITECTURE.md` §10.2.
//! * Per-pixel alpha needs `VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR`, which
//!   Mesa does advertise on Wayland. This is the platform where transparency
//!   is *not* the risk; X11 is.
//!
//! **Events arrive on C callbacks with no context**, so every listener does one
//! thing: append to an event queue the loop drains. Reaching back into the
//! widget from a listener would re-enter code the loop itself called.
//!
//! ## Status
//!
//! Skeleton. This is milestone M0 in `ROADMAP.md` — the layer-shell + wgpu
//! spike — because it is both the highest-value and the least proven seam in
//! the project.

use crate::Capabilities;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toplevel_fallback_admits_what_it_lost() {
        assert!(Shape::Layer.capabilities().degradation().is_none());
        assert!(Shape::Toplevel.capabilities().degradation().is_some());
    }
}
