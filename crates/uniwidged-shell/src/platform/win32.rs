//! The widget's window on Windows.
//!
//! Adapted from Ephemeris `src/win/window.rs`.
//!
//! It behaves like a Vista gadget, through five specific flags:
//!
//! * `WS_EX_NOREDIRECTIONBITMAP` — the prerequisite for a DirectComposition
//!   surface with true per-pixel alpha. Must be set at `CreateWindowEx` time;
//!   it cannot be added afterwards, which is one of the reasons this crate
//!   does not use a windowing abstraction.
//! * `WS_EX_TOOLWINDOW` — no entry in the taskbar or in Alt-Tab.
//! * `WS_EX_NOACTIVATE` — clicking the widget never takes focus away from the
//!   application you are working in. This is exactly what separates a gadget
//!   from a window in the way.
//! * `HWND_BOTTOM`, forced in `WM_WINDOWPOSCHANGING` — below every ordinary
//!   window, above the desktop, and therefore still clickable.
//! * `WS_POPUP` without `WS_THICKFRAME` — the system provides neither dragging
//!   nor resizing, so the widget provides its own.
//!
//! ## What is new here, relative to Ephemeris
//!
//! Ephemeris rendered with Direct2D into the DirectComposition surface.
//! uniwidged creates the composition visual itself and gives wgpu a
//! `SurfaceTargetUnsafe::CompositionVisual`, so Vello renders into a DXGI
//! composition swapchain that DWM blends per-pixel against the wallpaper.
//!
//! This is also where a backdrop is nearly free: `DwmSetWindowAttribute` with
//! `DWMWA_SYSTEMBACKDROP_TYPE` gives the widget Mica or an acrylic transient
//! backdrop for one call, which is a better blur than anything drawn.
//!
//! DPI is per-monitor v2 (`SetProcessDpiAwarenessContext`), resolved once at
//! the surface as everywhere else.
//!
//! ## Status
//!
//! Skeleton. Milestone M5.
