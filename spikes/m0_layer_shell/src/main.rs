//! M0/M1 · A desktop-layer surface, driven by the real renderer.
//!
//! Originally the M0 spike: does a `wlr-layer-shell` surface accept a GPU
//! swapchain with per-pixel alpha? It did, and the answer is written up in
//! `FINDINGS.md`.
//!
//! Since M1 it does a second job. The device, the surfaces and the
//! presentation all live in `uniwidged-render` now, so this program is a
//! standing integration test of that crate against a real compositor — and it
//! renders the *same scene twice*, once to the layer surface and once to a
//! readback surface, so `compare.py` can check that the headless path and the
//! on-screen path produce the same pixels. That equivalence is what makes a
//! golden image captured in CI evidence about what actually ships.
//!
//! Run it, then screenshot the compositor and run `verify.py`, `stacking.py`
//! and `compare.py` on the result — or just `./run.sh`.

mod scene;
mod wayland;

use std::path::Path;
use std::time::Duration;

use uniwidged_render::{Presentation, Renderer};

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
    println!("uniwidged M0/M1 spike — layer shell + uniwidged-render\n");

    let (conn, mut queue, state, surface) = wayland::connect(WIDTH, HEIGHT)?;

    println!("  compositor globals: {}", state.globals.len());
    for wanted in [
        "wl_compositor",
        "zwlr_layer_shell_v1",
        "xdg_wm_base",
        "zwp_linux_dmabuf_v1",
        "wl_shm",
    ] {
        match state.globals.iter().find(|(name, _)| name == wanted) {
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

    // The seam. The surface is configured and carries no buffer; the renderer
    // takes it from here — `ARCHITECTURE.md` §10.2.
    let (display_handle, window_handle) = surface.raw_handles(&conn);
    let (mut renderer, screen) = unsafe {
        Renderer::new_compatible_with(
            display_handle,
            window_handle,
            surface.size.0,
            surface.size.1,
        )
    }
    .map_err(|e| e.to_string())?;

    let info = renderer.adapter_info();
    println!("\n  adapter: {} ({:?})", info.name, info.backend);
    match renderer.alpha_mode(screen) {
        Some(mode) => {
            println!("  alpha mode: {mode:?}");
            let transparent = !matches!(mode, uniwidged_render::wgpu::CompositeAlphaMode::Opaque);
            println!(
                "    {}",
                if transparent {
                    "per-pixel alpha is available on this surface"
                } else {
                    "NO per-pixel alpha — the panel will sit on an opaque rectangle"
                }
            );
        }
        None => println!("  alpha mode: none (not a swapchain surface)"),
    }
    assert_eq!(renderer.presentation(screen), Some(Presentation::Swapchain));

    // The same scene, a second time, through the headless path. Two surfaces,
    // one device, one `render` call — which is also M1's whole point.
    let offscreen = renderer
        .add_readback_surface(surface.size.0, surface.size.1)
        .map_err(|e| e.to_string())?;

    let picture = scene::widget_scene(surface.size.0 as f64, surface.size.1 as f64);
    let stats = renderer
        .render(&[(screen, &picture), (offscreen, &picture)])
        .map_err(|e| e.to_string())?;
    println!(
        "\n  frame: {} rendered, {} skipped, {} submits",
        stats.rendered, stats.skipped, stats.submits
    );

    let png = std::env::var("SPIKE_READBACK_PNG").unwrap_or_else(|_| "readback.png".to_string());
    renderer
        .save_png(offscreen, Path::new(&png))
        .map_err(|e| e.to_string())?;
    println!("  the same scene, rendered headlessly: {png}");

    // Nothing changed, so nothing should happen. The claim the whole cost
    // model rests on, asserted where a real compositor can contradict it.
    let idle = renderer
        .render(&[(screen, &picture), (offscreen, &picture)])
        .map_err(|e| e.to_string())?;
    println!(
        "  a second frame with nothing changed: {} rendered, {} submits",
        idle.rendered, idle.submits
    );
    assert_eq!(idle.submits, 0, "an idle desktop must not submit a frame");

    // Stay up so the screenshot has something to capture, answering the
    // compositor's callbacks meanwhile. `roundtrip` rather than
    // `blocking_dispatch`: a widget at rest receives no events at all — which
    // is the entire point of the wake model — so blocking until one arrives
    // blocks forever. The real loop takes its timeout from `Deadline` instead.
    let hold: u64 = std::env::var("SPIKE_HOLD_SECS")
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
