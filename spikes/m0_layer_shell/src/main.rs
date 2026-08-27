//! M0 · Does a desktop-layer surface accept a GPU swapchain with per-pixel
//! alpha?
//!
//! Run it, then screenshot the compositor and run `verify.py` on the result.
//! The program's own output answers the two open architecture questions; the
//! screenshot is what proves it rather than reporting it.

mod render;
mod wayland;

use std::time::Duration;

const WIDTH: u32 = 360;
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
    println!("uniwidged M0 spike — layer shell + wgpu + Vello\n");

    let (conn, mut queue, state, surface) = wayland::connect(WIDTH, HEIGHT)?;

    println!("  compositor globals: {}", state.globals.len());
    for wanted in [
        "wl_compositor",
        "zwlr_layer_shell_v1",
        "xdg_wm_base",
        "zwp_linux_dmabuf_v1",
        "wl_shm",
    ] {
        let found = state.globals.iter().find(|(name, _)| name == wanted);
        match found {
            Some((_, v)) => println!("    [ok]      {wanted} (v{v})"),
            None => println!("    [missing] {wanted}"),
        }
    }

    println!("\n  surface role: {:?}", surface.shape);
    match surface.shape.degradation() {
        None => println!("    below every window, above the wallpaper, never focused"),
        // Ephemeris's rule, kept: a widget that stops doing the one thing it is
        // for says which, at start-up, rather than degrading quietly.
        Some(lost) => println!("    DEGRADED — {lost}"),
    }
    println!("    size: {}x{}", surface.size.0, surface.size.1);

    // The seam. The surface is configured and carries no buffer; wgpu takes it
    // from here. ARCHITECTURE.md §10.2 is answered by whether this returns.
    let (display_handle, window_handle) = surface.raw_handles(&conn);
    let mut renderer = unsafe {
        render::Renderer::new(
            display_handle,
            window_handle,
            surface.size.0,
            surface.size.1,
        )
    }?;

    let r = &renderer.report;
    println!("\n  adapter: {} ({})", r.adapter, r.backend);
    println!("  swapchain format: {:?}", r.format);
    println!("  alpha modes offered: {:?}", r.alpha_modes);
    println!("  alpha mode chosen:   {:?}", r.chosen_alpha);
    let transparent = matches!(
        r.chosen_alpha,
        vello::wgpu::CompositeAlphaMode::PreMultiplied
            | vello::wgpu::CompositeAlphaMode::PostMultiplied
            | vello::wgpu::CompositeAlphaMode::Inherit
    );
    if transparent {
        println!("    per-pixel alpha is available on this surface");
    } else {
        println!("    NO per-pixel alpha — the panel will sit on an opaque rectangle");
    }

    let scene = render::widget_scene(surface.size.0 as f64, surface.size.1 as f64);
    renderer.render(&scene)?;
    println!("\n  frame presented");

    // Stay up so the screenshot has something to capture, answering the
    // compositor's callbacks meanwhile. A widget that stops answering gets
    // killed for being unresponsive, which looks exactly like a rendering bug.
    //
    // `roundtrip` rather than `blocking_dispatch`: a widget at rest receives no
    // events at all — which is the entire point of the wake model — so blocking
    // until one arrives blocks forever. This is the same shape the real event
    // loop takes, where the timeout comes from `Deadline` instead.
    let hold = std::env::var("SPIKE_HOLD_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let deadline = std::time::Instant::now() + Duration::from_secs(hold);
    let mut state = state;
    while std::time::Instant::now() < deadline {
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("dispatch failed: {e}"))?;
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("  held for {hold}s, exiting cleanly");
    Ok(())
}
