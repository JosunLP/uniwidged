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
//! ## The transparency problem — the project's highest technical risk
//!
//! Ephemeris got a transparent X11 window by drawing into that 32-bit visual
//! with Cairo, and Cairo does not care what the GPU thinks. A Vulkan swapchain
//! does: on X11 it commonly advertises only
//! `VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR`, which would put an opaque rectangle
//! where the wallpaper should be — and an opaque rectangle is not a desktop
//! widget, it is a window with no title bar.
//!
//! Three mitigations, in the order they are tried:
//!
//! 1. Query for `PRE_MULTIPLIED` / `INHERIT` composite alpha and use it where
//!    the driver offers it.
//! 2. An EGL/GL path with an ARGB visual. EGL on X11 does honour the visual's
//!    alpha channel.
//! 3. `vello_cpu` into shared memory, composited with XRender — Ephemeris's own
//!    path, but driven from the *same* `vello::Scene`, so the output stays
//!    pixel-identical to the GPU path and the golden images still apply.
//!
//! If all three fail, X11 takes the CPU path permanently and nothing else about
//! the architecture changes. That is precisely why `uniwidged-compose` emits a
//! scene rather than talking to wgpu. See `ARCHITECTURE.md` §10.1.
//!
//! ## The loop is a `poll`
//!
//! X11 has no timers of its own. The loop waits on the display's socket and on
//! the host's wake pipe, with a timeout taken from
//! [`uniwidged_core::Deadline`]. That is what keeps a widget at rest costing
//! nothing.
//!
//! ## Status
//!
//! Skeleton. Milestone M4.

use crate::Capabilities;

/// Which of the three transparency paths this session ended up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaPath {
    /// A Vulkan swapchain that accepts pre-multiplied alpha. Best case.
    VulkanComposite,
    /// EGL with an ARGB visual.
    EglArgb,
    /// `vello_cpu` + XRender. Always available.
    CpuComposite,
}

impl AlphaPath {
    pub fn capabilities(self) -> Capabilities {
        Capabilities {
            gpu_swapchain: !matches!(self, AlphaPath::CpuComposite),
            ..Capabilities::FULL
        }
    }
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
            AlphaPath::CpuComposite,
        ] {
            assert!(path.capabilities().per_pixel_alpha, "{path:?}");
            assert!(path.capabilities().below_windows, "{path:?}");
        }
        assert!(!AlphaPath::CpuComposite.capabilities().gpu_swapchain);
    }
}
