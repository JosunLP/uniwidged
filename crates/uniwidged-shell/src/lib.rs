//! The only crate here that knows an operating system exists.
//!
//! Its one job is to produce a surface that behaves like a **desktop widget**
//! rather than an application window, and to hand a raw handle for it to
//! wgpu. Five properties define that behaviour, and every platform provides
//! them differently:
//!
//! | | Windows | macOS | Wayland | X11 |
//! |---|---|---|---|---|
//! | below windows, above desktop | `HWND_BOTTOM` forced in `WM_WINDOWPOSCHANGING` | `kCGDesktopIconWindowLevel + 1` | `wlr-layer-shell`, `BOTTOM` | `_NET_WM_STATE_BELOW` |
//! | never takes focus | `WS_EX_NOACTIVATE` | `canBecomeKeyWindow → NO` | `keyboard-interactivity = none` | `WM_HINTS.input = False` |
//! | no taskbar, no Alt-Tab | `WS_EX_TOOLWINDOW` | `.ignoresCycle`, `.accessory` | (the layer has neither) | `_SKIP_TASKBAR`, `_SKIP_PAGER` |
//! | on every desktop | (bottom-most) | `.canJoinAllSpaces`, `.stationary` | (per output) | `_NET_WM_STATE_STICKY` |
//! | per-pixel alpha | `WS_EX_NOREDIRECTIONBITMAP` + DirectComposition | borderless, `opaque = NO` | ARGB8888 | 32-bit TrueColor visual |
//!
//! ## Why not winit
//!
//! There is no winit in this workspace, which is unusual for a Rust GUI
//! project and is a deliberate choice rather than an oversight. winit has no
//! `wlr-layer-shell` surface role and no way to ask for one, and that role is
//! the entire definition of a desktop widget on Wayland. The same is true in
//! smaller ways elsewhere: `WS_EX_NOREDIRECTIONBITMAP` has to be set at
//! `CreateWindowEx` time, and `canBecomeKeyWindow` needs an `NSWindow`
//! subclass. A toolkit that abstracts windows abstracts away exactly the
//! properties this crate exists to set.
//!
//! Ferrum UI keeps its winit shell for ordinary application windows. uniwidged
//! uses everything in Ferrum UI *except* that.
//!
//! ## Provenance
//!
//! Every platform module here is adapted from the corresponding file in
//! [Ephemeris](https://github.com/JosunLP/Ephemeris), relicensed by its
//! copyright holder — see `NOTICE.md`. Each module names its upstream file.
//! What is new is the last row of the table above: Ephemeris drew into these
//! surfaces with Cairo, Direct2D and Core Graphics, and uniwidged hands them
//! to wgpu instead.

#![allow(dead_code)]

pub mod platform;

use uniwidged_core::geom::PhysicalRect;

/// Where a widget sits, and on which output.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// In physical pixels, in the output's own coordinate space.
    pub rect: PhysicalRect,
    /// The output's name as the platform reports it — `DP-1`, `\\.\DISPLAY1`.
    /// Stored rather than an index, because indices renumber when a monitor is
    /// unplugged and a widget that moves when you dock is a widget that has
    /// lost its place.
    pub output: Option<String>,
    /// Device pixels per logical pixel. The one place in the project where
    /// this number lives; see `uniwidged_core::geom`.
    pub scale: f64,
}

/// What the platform could actually deliver.
///
/// Reported at start-up and never guessed at. GNOME's Mutter has no layer
/// shell and has said it will not add one, so a widget there sits among the
/// windows rather than behind them and cannot place itself. Ephemeris's rule
/// applies unchanged: a widget that stops doing the one thing it is for should
/// say which, in the log, at start-up. Degrading quietly was rejected there and
/// is rejected here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Can the surface be kept below ordinary windows?
    pub below_windows: bool,
    /// Can the client choose its own position?
    pub self_placing: bool,
    /// Does the surface composite with per-pixel alpha against the wallpaper?
    pub per_pixel_alpha: bool,
    /// Is a GPU swapchain usable, or must this surface take the CPU path?
    pub gpu_swapchain: bool,
    /// Does the platform offer a backdrop blur?
    pub backdrop_blur: bool,
}

impl Capabilities {
    /// Everything works. What `wlr-layer-shell`, Win32 and AppKit all deliver.
    pub const FULL: Self = Self {
        below_windows: true,
        self_placing: true,
        per_pixel_alpha: true,
        gpu_swapchain: true,
        backdrop_blur: false,
    };

    /// A one-line summary for the start-up log, naming what is missing.
    ///
    /// Returns `None` when nothing is. The wording is deliberately concrete —
    /// "no layer shell: the widget sits among windows" is actionable, and
    /// "reduced functionality" is not.
    pub fn degradation(&self) -> Option<String> {
        let mut missing = Vec::new();
        if !self.below_windows {
            missing.push("cannot stay below ordinary windows");
        }
        if !self.self_placing {
            missing.push("cannot place itself; the compositor decides");
        }
        if !self.per_pixel_alpha {
            missing.push("no per-pixel alpha; the panel gets an opaque backing");
        }
        if !self.gpu_swapchain {
            missing.push("no GPU swapchain; rendering falls back to the CPU");
        }
        if missing.is_empty() {
            None
        } else {
            Some(missing.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_platform_reports_nothing_missing() {
        assert_eq!(Capabilities::FULL.degradation(), None);
    }

    #[test]
    fn mutter_is_described_concretely_rather_than_apologetically() {
        // What a GNOME session actually gets. The log line has to say which
        // capability is gone, because "reduced functionality" tells a user
        // nothing they can act on.
        let mutter = Capabilities {
            below_windows: false,
            self_placing: false,
            ..Capabilities::FULL
        };
        let line = mutter.degradation().expect("something is missing");
        assert!(line.contains("below ordinary windows"));
        assert!(line.contains("place itself"));
    }
}
