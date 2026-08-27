//! The widget's window on macOS.
//!
//! Adapted from Ephemeris `src/unix/mac/window.rs`.
//!
//! * **Level** `kCGDesktopIconWindowLevel + 1` — below every ordinary window,
//!   above the desktop and its icons, and still clickable. This is the one
//!   property the widget is defined by.
//! * **`canBecomeKeyWindow` returns `NO`** — clicking never takes focus from
//!   what you are working in. Needs an `NSWindow` subclass, which is the second
//!   reason there is no windowing abstraction in this crate.
//! * **Collection behaviour** `.canJoinAllSpaces | .stationary | .ignoresCycle`
//!   — follows you between Spaces, does not slide away under Mission Control,
//!   never appears in the switcher.
//! * **Activation policy** `.accessory` — no Dock tile, no menu bar. Set in
//!   code so a bare binary behaves the same as a bundled `.app`.
//! * **Borderless, `opaque = NO`, `hasShadow = NO`** — the renderer draws the
//!   glass, the corners and the shadow itself, so they match the other three
//!   platforms rather than matching AppKit.
//!
//! ## What is new here, relative to Ephemeris
//!
//! Ephemeris drew with Core Graphics into the view. uniwidged makes the
//! `NSView` layer-backed with a `CAMetalLayer` (`isOpaque = false`) and hands
//! wgpu a `SurfaceTargetUnsafe::CoreAnimationLayer`. `NSVisualEffectView`
//! behind the Metal layer gives a real backdrop blur.
//!
//! ## Coordinates
//!
//! AppKit measures screens from the bottom left; the rest of this project
//! measures from the top left. The conversion happens here, at the two
//! functions that touch a screen rectangle, so a saved placement means the same
//! thing on every platform.
//!
//! ## Status
//!
//! Skeleton. Milestone M6.
