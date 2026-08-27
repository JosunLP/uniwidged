//! The desktop, running on X11.
//!
//! This is `ARCHITECTURE.md` §5's loop with a real window system under it:
//!
//! ```text
//! loop {
//!     let timeout = host.next_timeout(now);   // None ⇒ block
//!     window.wait(timeout);                   // costs nothing
//!     deliver whatever arrived
//!     host.tick(now, wall);
//! }
//! ```
//!
//! One process, one GPU device, one loop, N widgets — each with its own
//! desktop-layer window and its own swapchain, all rendered from the same
//! device.
//!
//! Wayland is not wired up here yet: its window code is in
//! `spikes/m0_layer_shell` and still has to be promoted into
//! `uniwidged-shell`, which is the next platform job rather than a missing
//! design. On a Wayland session this runs through XWayland, and says so.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use uniwidged_compose::Theme;
use uniwidged_core::geom::PhysicalRect;
use uniwidged_core::{Point, WidgetId};
use uniwidged_host::Host;
use uniwidged_host::config::Config;
use uniwidged_runtime::manifest::Manifest;
use uniwidged_shell::platform::x11::{AlphaPath, Input, X11Window};

/// What `uniwidged doctor` can say about an X11 session without starting one.
pub fn doctor() {
    match std::env::var("WAYLAND_DISPLAY") {
        Ok(display) if !display.is_empty() => {
            println!("  session: Wayland ({display})");
            println!(
                "    this build has no native Wayland shell yet, so widgets run \
                 through XWayland — whether they stay below other windows is \
                 then the compositor's decision, and several ignore it"
            );
        }
        _ => println!("  session: X11"),
    }
    match X11Window::open(PhysicalRect::new(-200, -200, 32, 32), "uniwidged doctor") {
        Ok(window) => {
            println!("  32-bit TrueColor visual: yes");
            println!(
                "  compositing manager: {}",
                if window.has_compositor() {
                    "present"
                } else {
                    "ABSENT — the X server will ignore the alpha channel \
                     entirely, and widgets will be opaque rectangles"
                }
            );
            let caps = window.capabilities(AlphaPath::VulkanComposite);
            match caps.degradation() {
                None => println!("  everything a widget needs is available"),
                Some(lost) => println!("  degraded: {lost}"),
            }
        }
        Err(e) => println!("  cannot open a window: {e}"),
    }
}

/// Starts the desktop and does not return until it is closed.
pub fn run(config_path: &Path) -> Result<(), String> {
    let config = Config::read(config_path).map_err(|e| e.to_string())?;
    let enabled: Vec<_> = config.widgets.iter().filter(|w| w.enabled).collect();
    if enabled.is_empty() {
        return Err(format!(
            "nothing to run: {} lists no enabled widgets. \
             Install one with `uniwidged install <widget.wasm>`.",
            config_path.display()
        ));
    }

    let mut host =
        Host::new(Theme::default(), Duration::from_millis(16)).map_err(|e| e.to_string())?;
    // Every widget's window, kept alive for as long as the desktop is: dropping
    // one destroys the window and, with it, the surface wgpu is drawing into.
    let mut windows: Vec<X11Window> = Vec::new();
    let mut by_window: HashMap<u32, WidgetId> = HashMap::new();

    for widget in &enabled {
        let wasm = std::fs::read(&widget.component)
            .map_err(|e| format!("{}: {e}", widget.component.display()))?;
        let manifest_text = std::fs::read_to_string(widget.manifest_path())
            .map_err(|e| format!("{}: {e}", widget.manifest_path().display()))?;
        let manifest = Manifest::parse(&manifest_text).map_err(|e| e.to_string())?;
        let policy = widget.policy(&manifest).map_err(|e| e.to_string())?;

        let placement = &widget.placement;
        let window = X11Window::open(
            PhysicalRect::new(placement.x, placement.y, placement.width, placement.height),
            &widget.name,
        )
        .map_err(|e| e.to_string())?;
        if !window.has_compositor() {
            eprintln!(
                "uniwidged: no compositing manager is running, so `{}` will be \
                 an opaque rectangle rather than a widget",
                widget.name
            );
        }

        let id = host
            .install(&wasm, manifest, policy)
            .map_err(|e| e.to_string())?;
        let (display, handle) = window.raw_handles();
        // SAFETY: the window is kept in `windows` for the lifetime of the
        // host, and was configured and mapped above.
        let surface = unsafe {
            host.renderer_mut().add_swapchain_surface(
                display,
                handle,
                placement.width,
                placement.height,
            )
        }
        .map_err(|e| e.to_string())?;
        host.attach(id, surface).map_err(|e| e.to_string())?;

        if host.renderer_mut().alpha_mode(surface)
            == Some(uniwidged_render::wgpu::CompositeAlphaMode::Opaque)
        {
            eprintln!(
                "uniwidged: this driver offers no transparent swapchain, so `{}` \
                 will sit on a solid rectangle — see ARCHITECTURE.md §10.1",
                widget.name
            );
        }

        by_window.insert(window.window_id(), id);
        windows.push(window);
        println!(
            "  {} at {},{} {}x{}",
            widget.name, placement.x, placement.y, placement.width, placement.height
        );
    }

    println!("  {} widget(s) running\n", windows.len());

    // A wall-clock limit, so an automated run terminates. A desktop has none.
    let until = std::env::var("UNIWIDGED_RUN_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|s| Instant::now() + Duration::from_secs(s));

    loop {
        if until.is_some_and(|deadline| Instant::now() > deadline) {
            println!("  exiting cleanly");
            return Ok(());
        }
        let timeout = host.next_timeout(Instant::now());
        // Bounded only so that `UNIWIDGED_RUN_SECS` can end an idle desktop;
        // without that this would block indefinitely, which is the whole point.
        let timeout = match (timeout, until) {
            (None, None) => None,
            (t, _) => Some(t.unwrap_or(Duration::from_millis(250))),
        };
        windows[0].wait(timeout).map_err(|e| e.to_string())?;

        for window in &windows {
            for input in window.drain_events().map_err(|e| e.to_string())? {
                match input {
                    Input::Click { window, x, y } => {
                        if let Some(id) = by_window.get(&window) {
                            let _ = host.click(*id, Point::new(x, y));
                        }
                    }
                    Input::Exposed { window } => {
                        // An X11 window has no backing store, so the contents
                        // are gone and the damage check would say nothing
                        // changed. Only the host knows how to put it back.
                        if let Some(id) = by_window.get(&window) {
                            host.redraw(*id);
                        }
                    }
                    Input::Closed { .. } => {
                        println!("  closed");
                        return Ok(());
                    }
                }
            }
        }

        let report = host
            .tick(Instant::now(), SystemTime::now())
            .map_err(|e| e.to_string())?;
        for line in host.take_log() {
            println!("  {line}");
        }
        for id in &report.suspended {
            eprintln!(
                "uniwidged: giving up on `{}` after repeated failures; its last \
                 frame stays on screen",
                host.widget(*id).map(|w| w.name.as_str()).unwrap_or("?")
            );
        }
    }
}
