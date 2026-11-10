//! These tests draw for real.
//!
//! They need a Vulkan adapter, and a *software* one is enough — CI installs
//! `mesa-vulkan-drivers` and gets lavapipe, which is deterministic. Where no
//! adapter exists at all the tests skip loudly rather than failing, so that
//! `cargo test` on a machine with no driver is not a wall of red.
//!
//! A skip that nobody notices is a test that stopped running, though, so
//! setting `UNIWIDGED_REQUIRE_GPU=1` turns the skip into a failure. **CI sets
//! it**, which is what stops this file from quietly covering nothing.

use super::*;
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Fill, color::AlphaColor};

/// A renderer, or `None` where the machine has no adapter at all.
fn gpu() -> Option<Renderer> {
    match Renderer::new() {
        Ok(r) => Some(r),
        Err(e) => {
            if std::env::var_os("UNIWIDGED_REQUIRE_GPU").is_some() {
                panic!("UNIWIDGED_REQUIRE_GPU is set but there is no adapter: {e}");
            }
            eprintln!("SKIPPED: {e}");
            None
        }
    }
}

/// An opaque rectangle filling the whole surface, in one flat colour.
fn solid(width: f64, height: f64, colour: AlphaColor<vello::peniko::color::Srgb>) -> Scene {
    let mut scene = Scene::new();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        colour,
        None,
        &Rect::new(0.0, 0.0, width, height),
    );
    scene
}

/// A rectangle covering the top-left quarter, leaving the rest untouched.
fn quarter(width: f64, height: f64, colour: AlphaColor<vello::peniko::color::Srgb>) -> Scene {
    let mut scene = Scene::new();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        colour,
        None,
        &Rect::new(0.0, 0.0, width / 2.0, height / 2.0),
    );
    scene
}

#[test]
fn padded_rows_round_up_to_the_copy_alignment() {
    // Pure arithmetic, no GPU: `copy_texture_to_buffer` requires 256-byte rows,
    // and getting this wrong reads a shifted image that still looks plausible.
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    for width in [1u32, 63, 64, 65, 100, 360, 1920] {
        let padded = super::surface::padded_row_for_test(width);
        assert!(padded >= width * 4, "width {width} lost bytes");
        assert_eq!(padded % align, 0, "width {width} is not {align}-aligned");
        assert!(padded - width * 4 < align, "width {width} over-padded");
    }
}

#[test]
fn three_surfaces_of_different_sizes_render_in_one_frame() {
    // The M1 exit criterion. Three widgets, three sizes, three scenes, one
    // `render` call — and each one gets its own content, not its neighbour's.
    let Some(mut r) = gpu() else { return };

    let sizes = [(120u32, 80u32), (200, 200), (64, 300)];
    let colours = [
        AlphaColor::from_rgba8(220, 40, 40, 255),
        AlphaColor::from_rgba8(40, 220, 40, 255),
        AlphaColor::from_rgba8(40, 40, 220, 255),
    ];
    let ids: Vec<_> = sizes
        .iter()
        .map(|(w, h)| r.add_readback_surface(*w, *h).expect("surface"))
        .collect();

    let scenes: Vec<_> = sizes
        .iter()
        .zip(colours)
        .map(|((w, h), c)| solid(*w as f64, *h as f64, c))
        .collect();
    let batch: Vec<_> = ids.iter().copied().zip(scenes.iter()).collect();

    let stats = r.render(&batch).expect("frame");
    assert_eq!(stats.rendered, 3);
    assert_eq!(stats.skipped, 0);
    // One Vello submit per surface, plus one for every copy together. The
    // module note explains why the second half cannot also be per-surface.
    assert_eq!(stats.submits, 4, "unexpected submit count: {stats:?}");

    for (i, id) in ids.iter().enumerate() {
        let image = r.read_pixels(*id).expect("pixels");
        let (w, h) = sizes[i];
        assert_eq!((image.width, image.height), (w, h));
        let (cr, cg, cb, ca) = image.pixel(w / 2, h / 2).expect("centre pixel");
        let expected = colours[i].to_rgba8();
        assert_eq!(ca, 255, "surface {i} centre should be opaque");
        // Software rasterisation is exact for a flat fill, but leave room for
        // the sRGB round trip rather than demanding bit equality.
        for (got, want) in [(cr, expected.r), (cg, expected.g), (cb, expected.b)] {
            assert!(
                got.abs_diff(want) <= 2,
                "surface {i}: got rgb({cr},{cg},{cb}), wanted rgb({},{},{})",
                expected.r,
                expected.g,
                expected.b
            );
        }
    }
}

#[test]
fn a_clean_surface_costs_nothing() {
    // The entire cost model. Eight widgets of which one ticked must cost one
    // widget's worth of work.
    let Some(mut r) = gpu() else { return };
    let a = r.add_readback_surface(64, 64).unwrap();
    let b = r.add_readback_surface(64, 64).unwrap();
    let scene = solid(64.0, 64.0, AlphaColor::from_rgba8(10, 20, 30, 255));

    // Both start dirty: a surface that has never been drawn has no frame yet.
    let first = r.render(&[(a, &scene), (b, &scene)]).unwrap();
    assert_eq!(first.rendered, 2);
    assert!(!r.has_work(), "everything drawn should now be clean");

    // Nothing changed: no rasterisation, no submit at all.
    let idle = r.render(&[(a, &scene), (b, &scene)]).unwrap();
    assert_eq!(idle.rendered, 0);
    assert_eq!(idle.submits, 0, "an idle desktop must not submit a frame");
    assert_eq!(idle.skipped, 2);

    // One widget ticks.
    r.mark_dirty(a);
    assert!(r.has_work());
    let one = r.render(&[(a, &scene), (b, &scene)]).unwrap();
    assert_eq!(one.rendered, 1);
    assert_eq!(one.skipped, 1);
    assert_eq!(one.submits, 2);
}

#[test]
fn undrawn_pixels_stay_fully_transparent() {
    // What makes a widget a widget: everywhere the scene does not draw, the
    // wallpaper must show through untouched. If the base colour were ever an
    // opaque black, this is where it would show.
    let Some(mut r) = gpu() else { return };
    let id = r.add_readback_surface(100, 100).unwrap();
    let scene = quarter(100.0, 100.0, AlphaColor::from_rgba8(255, 255, 255, 255));
    r.render(&[(id, &scene)]).unwrap();

    let image = r.read_pixels(id).unwrap();
    let (_, _, _, drawn_alpha) = image.pixel(10, 10).unwrap();
    let (br, bg, bb, ba) = image.pixel(90, 90).unwrap();
    assert_eq!(drawn_alpha, 255, "the drawn quarter should be opaque");
    assert_eq!(
        (br, bg, bb, ba),
        (0, 0, 0, 0),
        "an undrawn pixel must be fully transparent, not opaque black"
    );
}

#[test]
fn a_resized_surface_is_redrawn_at_its_new_size() {
    let Some(mut r) = gpu() else { return };
    let id = r.add_readback_surface(50, 50).unwrap();
    let scene = solid(50.0, 50.0, AlphaColor::from_rgba8(200, 100, 50, 255));
    r.render(&[(id, &scene)]).unwrap();
    assert!(!r.has_work());

    r.resize(id, 120, 90).unwrap();
    assert_eq!(r.size(id), Some((120, 90)));
    // The old frame is the wrong size, so the surface must have gone dirty by
    // itself — nothing else in the host knows to mark it.
    assert!(r.has_work(), "a resize must schedule a redraw");

    let bigger = solid(120.0, 90.0, AlphaColor::from_rgba8(200, 100, 50, 255));
    r.render(&[(id, &bigger)]).unwrap();
    let image = r.read_pixels(id).unwrap();
    assert_eq!((image.width, image.height), (120, 90));
    assert_eq!(image.data.len(), 120 * 90 * 4);
    assert_eq!(image.pixel(119, 89).unwrap().3, 255);
}

#[test]
fn a_dirty_surface_with_no_scene_this_frame_stays_dirty() {
    // The widget's view has not been rebuilt yet. Clearing the flag here would
    // drop the update on the floor and leave a stale panel forever.
    let Some(mut r) = gpu() else { return };
    let a = r.add_readback_surface(32, 32).unwrap();
    let b = r.add_readback_surface(32, 32).unwrap();
    let scene = solid(32.0, 32.0, AlphaColor::from_rgba8(1, 2, 3, 255));

    let stats = r.render(&[(a, &scene)]).unwrap();
    assert_eq!(stats.rendered, 1);
    assert!(r.has_work(), "b was never drawn and must still be pending");

    let stats = r.render(&[(b, &scene)]).unwrap();
    assert_eq!(stats.rendered, 1);
    assert!(!r.has_work());
}

#[test]
fn removing_a_surface_removes_its_work() {
    let Some(mut r) = gpu() else { return };
    let id = r.add_readback_surface(16, 16).unwrap();
    assert_eq!(r.surface_count(), 1);
    assert!(r.has_work());
    r.remove_surface(id);
    assert_eq!(r.surface_count(), 0);
    assert!(!r.has_work());
    assert!(matches!(
        r.read_pixels(id),
        Err(RenderError::NoSuchSurface(_))
    ));
}

#[test]
fn a_surface_reports_how_it_will_be_presented() {
    // The host branches on this: readback pixels must be handed to the
    // platform, swapchain pixels must not be touched.
    let Some(mut r) = gpu() else { return };
    let id = r.add_readback_surface(8, 8).unwrap();
    assert_eq!(r.presentation(id), Some(Presentation::Readback));
    assert_eq!(r.presentation(SurfaceId(999)), None);
}

#[test]
fn a_readback_surface_reports_no_alpha_mode() {
    // There is no swapchain to have one, and returning `Auto` here would read
    // as "the platform chose", which is a different and misleading claim.
    let Some(mut r) = gpu() else { return };
    let id = r.add_readback_surface(8, 8).unwrap();
    assert_eq!(r.alpha_mode(id), None);
}

#[test]
fn readback_carries_straight_alpha() {
    // Vello's `fine.wgsl` divides by alpha before writing the texture, so what
    // comes back is *straight* alpha — which is also what PNG stores, so the
    // golden-image path converts nothing.
    //
    // This is worth a test because the first guess was the opposite, and being
    // wrong about it silently produces images that look almost right. See
    // `blit.rs` for the half of the mismatch that actually bit.
    let Some(mut r) = gpu() else { return };
    let id = r.add_readback_surface(16, 16).unwrap();
    // White at half alpha: straight it is (255,255,255,128), premultiplied it
    // would be (128,128,128,128). The two cannot be confused.
    let scene = solid(16.0, 16.0, AlphaColor::from_rgba8(255, 255, 255, 128));
    r.render(&[(id, &scene)]).unwrap();

    let image = r.read_pixels(id).unwrap();
    let (pr, pg, pb, pa) = image.pixel(8, 8).unwrap();
    assert!(pa.abs_diff(128) <= 1, "expected half alpha, got {pa}");
    for c in [pr, pg, pb] {
        assert!(
            c.abs_diff(255) <= 2,
            "read_pixels should be straight alpha: channel {c} at alpha {pa}"
        );
    }

    // And the conversion a compositor-facing caller needs.
    let premul = image.to_premultiplied();
    let i = ((8 * 16 + 8) * 4) as usize;
    assert!(premul[i + 3].abs_diff(128) <= 1, "alpha must be untouched");
    for c in &premul[i..i + 3] {
        assert!(
            c.abs_diff(128) <= 2,
            "premultiplied white at half alpha should be ~128, got {c}"
        );
    }
}

#[test]
fn premultiplying_is_exact_at_both_ends_of_the_range() {
    // Pure arithmetic. Rounding to nearest rather than truncating matters:
    // over a whole image, truncation is a visible darkening.
    let image = Image {
        width: 4,
        height: 1,
        data: vec![
            10, 20, 30, 255, // opaque: unchanged
            10, 20, 30, 0, // transparent: nothing survives
            128, 64, 32, 128, // half alpha: halved
            255, 255, 255, 3, // truncation would give 2, correct is 3
        ],
    };
    let premul = image.to_premultiplied();
    assert_eq!(&premul[0..4], &[10, 20, 30, 255]);
    assert_eq!(&premul[4..8], &[0, 0, 0, 0]);
    assert_eq!(&premul[8..12], &[64, 32, 16, 128]);
    assert_eq!(&premul[12..16], &[3, 3, 3, 3]);
}

#[test]
fn the_alpha_conversion_follows_the_surface_mode() {
    // The mapping that fixes the straight-versus-premultiplied mismatch. Auto
    // and Inherit mean "the platform decides", and every platform this runs on
    // decides premultiplied.
    use super::blit::AlphaConversion as C;
    use wgpu::CompositeAlphaMode as M;
    assert_eq!(C::for_mode(M::PreMultiplied), C::Premultiply);
    assert_eq!(C::for_mode(M::Auto), C::Premultiply);
    assert_eq!(C::for_mode(M::Inherit), C::Premultiply);
    // Core Animation composites every layer premultiplied, whatever wgpu's
    // Metal backend calls the mode — see `blit::AlphaConversion::for_mode`.
    #[cfg(target_os = "macos")]
    assert_eq!(C::for_mode(M::PostMultiplied), C::Premultiply);
    #[cfg(not(target_os = "macos"))]
    assert_eq!(C::for_mode(M::PostMultiplied), C::Straight);
    assert_eq!(C::for_mode(M::Opaque), C::Straight);
}
