//! The scene the spike draws.
//!
//! Everything that used to be in this file — the device, the surface, the
//! swapchain, the intermediate texture, the blitter — moved into
//! `uniwidged-render` at milestone M1. What is left is the part that was
//! always the spike's own: a picture to put on the screen.

use vello::Scene;
use vello::kurbo::{Affine, Point, Rect, RoundedRect, Stroke};
use vello::peniko::{ColorStop, Extend, Gradient, color::AlphaColor};

/// The panel, as a widget would look.
///
/// Every element here is one of the quality claims in `ARCHITECTURE.md` §7,
/// made concrete: a real Gaussian shadow rather than stacked strokes, a
/// gradient interpolated in premultiplied alpha, a sub-pixel hairline that
/// stays a hairline, and rounded caps that are actually round.
pub fn widget_scene(width: f64, height: f64) -> Scene {
    let mut scene = Scene::new();
    if std::env::var_os("SPIKE_FORCE_TOPLEVEL").is_some() {
        // An ordinary application window: fully opaque, edge to edge. Used to
        // prove the stacking order — if the layer surface really is on the
        // bottom layer, this covers it completely.
        scene.fill(
            vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            AlphaColor::from_rgba8(240, 90, 30, 255),
            None,
            &Rect::new(0.0, 0.0, width, height),
        );
        return scene;
    }
    let inset = 24.0;
    let body = RoundedRect::new(inset, inset, width - inset, height - inset, 18.0);

    // A real Gaussian blur. The shadow is what tells the eye the panel is
    // floating above the wallpaper rather than painted onto it.
    scene.draw_blurred_rounded_rect(
        Affine::translate((0.0, 6.0)),
        body.rect(),
        AlphaColor::from_rgba8(0, 0, 0, 110),
        18.0,
        14.0,
    );

    // The glass. Partly transparent on purpose: if the alpha never reaches the
    // compositor, this reads as flat grey and the spike has failed.
    let glass = Gradient::new_linear(Point::new(0.0, inset), Point::new(0.0, height - inset))
        .with_stops([
            ColorStop {
                offset: 0.0,
                color: AlphaColor::from_rgba8(38, 40, 48, 190).into(),
            },
            ColorStop {
                offset: 1.0,
                color: AlphaColor::from_rgba8(20, 21, 26, 150).into(),
            },
        ])
        .with_extend(Extend::Pad);
    scene.fill(
        vello::peniko::Fill::NonZero,
        Affine::IDENTITY,
        &glass,
        None,
        &body,
    );

    // A one-pixel specular highlight along the top edge. At any scale factor
    // this has to stay one pixel and stay crisp.
    scene.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        AlphaColor::from_rgba8(255, 255, 255, 46),
        None,
        &body,
    );

    // Three meters, the shape a real widget takes.
    let track_x = inset + 22.0;
    let track_w = width - inset * 2.0 - 44.0;
    for (i, (value, color)) in [
        (0.32f64, AlphaColor::from_rgba8(120, 190, 255, 255)),
        (0.61, AlphaColor::from_rgba8(120, 190, 255, 255)),
        (0.94, AlphaColor::from_rgba8(255, 110, 110, 255)),
    ]
    .into_iter()
    .enumerate()
    {
        let y = inset + 46.0 + i as f64 * 30.0;
        let track = RoundedRect::new(track_x, y, track_x + track_w, y + 8.0, 4.0);
        scene.fill(
            vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            AlphaColor::from_rgba8(255, 255, 255, 28),
            None,
            &track,
        );
        let filled = RoundedRect::new(track_x, y, track_x + track_w * value, y + 8.0, 4.0);
        scene.fill(
            vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            color,
            None,
            &filled,
        );
    }

    // A title bar stand-in. Text needs a font stack, which is M2's problem —
    // this is a placeholder shaped like one so the layout reads correctly.
    scene.fill(
        vello::peniko::Fill::NonZero,
        Affine::IDENTITY,
        AlphaColor::from_rgba8(255, 255, 255, 200),
        None,
        &Rect::new(track_x, inset + 22.0, track_x + 74.0, inset + 32.0),
    );

    scene
}
