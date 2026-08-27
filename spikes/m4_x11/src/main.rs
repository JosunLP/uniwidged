//! M4 · Is a widget transparent, and below windows, on X11?
//!
//! The Wayland answer was unambiguous (`spikes/m0_layer_shell`). X11 is where
//! `ARCHITECTURE.md` §10.1 says the risk actually lives: Ephemeris got a
//! transparent window by drawing into a 32-bit visual with Cairo, and Cairo
//! does not care what the GPU thinks. A Vulkan swapchain does.
//!
//! So this reports **which of the three paths the session took** rather than
//! assuming one, and renders a real composed widget — view tree through
//! `uniwidged-compose`, not a hand-drawn scene — so that what is on screen is
//! what a widget would actually look like.
//!
//! Run it, screenshot the display, and check the arithmetic with `verify.py`;
//! or just `./run.sh`.

use std::time::Duration;

use uniwidged_compose::{Compose, Compositor, Theme};
use uniwidged_compose::theme::Oklch;
use uniwidged_core::geom::PhysicalRect;
use uniwidged_core::{NodeId, Size, WidgetId};
use uniwidged_render::Renderer;
use uniwidged_shell::platform::x11::{AlphaPath, WindowKind, X11Window};
use uniwidged_view::build::*;
use uniwidged_view::{Accent, View};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 200;

fn main() {
    match run() {
        Ok(()) => {}
        Err(why) => {
            eprintln!("\n  FAILED: {why}\n");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<(), String> {
    println!("uniwidged M4 spike — X11 + uniwidged-render + uniwidged-compose\n");

    let ordinary = std::env::var_os("SPIKE_ORDINARY_WINDOW").is_some();
    let kind = if ordinary {
        WindowKind::Ordinary
    } else {
        WindowKind::DesktopWidget
    };
    let at = |key: &str, fallback: i32| {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(fallback)
    };
    let rect = PhysicalRect::new(
        at("SPIKE_X", 48),
        at("SPIKE_Y", 48),
        WIDTH,
        HEIGHT,
    );

    let window = X11Window::open_kind(rect, "uniwidged spike", kind).map_err(|e| e.to_string())?;
    println!("  window kind: {kind:?}");
    println!("  32-bit TrueColor visual: 0x{:x}", window.visual_id());
    println!(
        "  compositing manager: {}",
        if window.has_compositor() {
            "present"
        } else {
            "ABSENT — the X server will ignore the alpha channel entirely"
        }
    );

    let (display, handle) = window.raw_handles();
    // `SPIKE_FORCE_READBACK` exercises the fallback on a session where the
    // swapchain path works, which is the only way to keep it tested: a
    // fallback nobody has run is a fallback that has stopped working.
    let forced_readback = std::env::var_os("SPIKE_FORCE_READBACK").is_some();
    let (mut renderer, surface) = if forced_readback {
        let mut renderer = Renderer::new().map_err(|e| e.to_string())?;
        let surface = renderer
            .add_readback_surface(WIDTH, HEIGHT)
            .map_err(|e| e.to_string())?;
        (renderer, surface)
    } else {
        unsafe { Renderer::new_compatible_with(display, handle, WIDTH, HEIGHT) }
            .map_err(|e| e.to_string())?
    };

    let info = renderer.adapter_info();
    println!("\n  adapter: {} ({:?})", info.name, info.backend);
    let alpha = renderer.alpha_mode(surface);
    println!("  swapchain format: {:?}", renderer.swapchain_format(surface));
    println!("  alpha mode: {alpha:?}");

    // The question §10.1 exists to answer, answered by what the surface
    // actually offered rather than by what was hoped for.
    let path = if forced_readback {
        AlphaPath::Readback
    } else {
        match alpha {
            Some(uniwidged_render::wgpu::CompositeAlphaMode::PreMultiplied)
            | Some(uniwidged_render::wgpu::CompositeAlphaMode::PostMultiplied)
            | Some(uniwidged_render::wgpu::CompositeAlphaMode::Inherit) => {
                AlphaPath::VulkanComposite
            }
            // `EglArgb` is the one path still unbuilt; a session that lands
            // here today gets the readback path instead, which always works.
            _ => AlphaPath::Readback,
        }
    };
    println!("  alpha path: {}", path.describe());
    let caps = window.capabilities(path);
    match caps.degradation() {
        None => println!("    below every window, above the desktop, never focused"),
        Some(lost) => println!("    DEGRADED — {lost}"),
    }

    let mut compositor = Compositor::new(theme());
    let view = widget();
    let composed = compositor.compose(Compose::new(
        &view,
        WidgetId(1),
        "System load",
        Size::new(WIDTH as f64, HEIGHT as f64),
    ));
    // An ordinary application window is opaque, and the stacking test needs it
    // to be unambiguously so — a translucent stand-in cannot tell "covered"
    // from "still visible underneath". So it is a flat fill rather than a
    // themed panel, which is the one place this spike draws for itself.
    let scene = if ordinary { opaque_window() } else { composed.scene };
    // The same scene a second time, through the headless path, so that what is
    // on screen can be checked against what it *should* be rather than only
    // looked at. This is the technique the M0 spike introduced, and on X11 it
    // is not a nicety: the surface can report an alpha mode it does not
    // deliver, and only the pixels settle it.
    //
    // On the readback path the two are the same surface, because there is no
    // swapchain to be a second one.
    let offscreen = if forced_readback {
        surface
    } else {
        renderer
            .add_readback_surface(WIDTH, HEIGHT)
            .map_err(|e| e.to_string())?
    };
    let batch: Vec<_> = if forced_readback {
        vec![(surface, &scene)]
    } else {
        vec![(surface, &scene), (offscreen, &scene)]
    };
    renderer.render(&batch).map_err(|e| e.to_string())?;

    if forced_readback {
        // The fallback itself: rasterised on the GPU as always, read back, and
        // handed to the X server. The compositor blends it exactly as it would
        // a swapchain — which is why only *presentation* falls back here.
        let image = renderer.read_pixels(surface).map_err(|e| e.to_string())?;
        window
            .present_rgba(image.width, image.height, &image.data)
            .map_err(|e| e.to_string())?;
    }

    let png = std::env::var("SPIKE_READBACK_PNG").unwrap_or_else(|_| "readback.png".to_string());
    renderer
        .save_png(offscreen, std::path::Path::new(&png))
        .map_err(|e| e.to_string())?;
    println!("\n  frame presented");
    println!("  the same scene, rendered headlessly: {png}");

    let idle = renderer
        .render(&[(surface, &scene)])
        .map_err(|e| e.to_string())?;
    println!(
        "  a second frame with nothing changed: {} rendered, {} submits",
        idle.rendered, idle.submits
    );

    let hold: u64 = std::env::var("SPIKE_HOLD_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let deadline = std::time::Instant::now() + Duration::from_secs(hold);
    while std::time::Instant::now() < deadline {
        if forced_readback {
            // An X11 window has no backing store by default, so anything that
            // exposes it loses the contents. A real host redraws on `Expose`;
            // the spike simply puts the frame back.
            let image = renderer.read_pixels(surface).map_err(|e| e.to_string())?;
            window
                .present_rgba(image.width, image.height, &image.data)
                .map_err(|e| e.to_string())?;
        }
        window.flush().map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("  held for {hold}s, exiting cleanly");
    Ok(())
}

/// A flat, fully opaque fill: what an application window looks like from the
/// outside, for the stacking test.
fn opaque_window() -> uniwidged_render::Scene {
    use uniwidged_render::kurbo::{Affine, Rect};
    use uniwidged_render::peniko::{Fill, color::AlphaColor};
    let mut scene = uniwidged_render::Scene::new();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        AlphaColor::from_rgba8(240, 90, 30, 255),
        None,
        &Rect::new(0.0, 0.0, WIDTH as f64, HEIGHT as f64),
    );
    scene
}

/// A fixed theme, so the screenshot does not depend on a desktop accent colour.
fn theme() -> Theme {
    Theme {
        brand: Oklch::new(0.62, 0.14, 250.0),
        ..Theme::default()
    }
}

/// The same widget the golden images use, so what is on screen can be compared
/// against a reference rendered headlessly.
fn widget() -> View {
    panel(NodeId::named("root"))
        .gap(2)
        .child(
            row()
                .child(text("System").role(Role::Title))
                .child(spacer())
                .child(chip("live").accent(Accent::Positive))
                .child(
                    icon(Icon::Refresh)
                        .on_click(NodeId::named("refresh"))
                        .label("Refresh"),
                ),
        )
        .child(divider())
        .child(meter("CPU", 0.32, Accent::Neutral))
        .child(meter("Memory", 0.78, Accent::Warning))
        .child(meter("Disk", 0.94, Accent::Critical))
        .view()
}

fn meter(name: &str, value: f32, accent: Accent) -> Node {
    column()
        .child(
            row()
                .child(text(name).role(Role::Caption))
                .child(spacer())
                .child(
                    text(format!("{:.0}%", value * 100.0))
                        .role(Role::Caption)
                        .accent(accent),
                ),
        )
        .child(progress(value).accent(accent))
}
