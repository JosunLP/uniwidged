//! The theme is pure arithmetic, so all of it is testable with no GPU, no
//! window and no font.

use super::*;

/// sRGB back to Oklch, so a conversion can be checked for what it *preserved*
/// rather than only for what it produced. Only the tests need this direction.
#[allow(clippy::excessive_precision)]
fn srgb_to_oklch(r: u8, g: u8, b: u8) -> Oklch {
    let linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (linear(r), linear(g), linear(b));

    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();

    let lightness = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;

    let hue = bb.atan2(a).to_degrees().rem_euclid(360.0);
    Oklch::new(lightness, (a * a + bb * bb).sqrt(), hue)
}

fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 { 360.0 - d } else { d }
}

#[test]
fn the_ends_of_the_lightness_range_are_black_and_white() {
    assert_eq!(Oklch::new(0.0, 0.0, 0.0).to_srgb(), (0, 0, 0));
    assert_eq!(Oklch::new(1.0, 0.0, 0.0).to_srgb(), (255, 255, 255));
}

#[test]
fn a_colour_survives_the_round_trip() {
    // Every in-gamut colour must come back as itself, or nothing built on the
    // conversion means anything.
    for original in [
        Oklch::new(0.62, 0.14, 250.0),
        Oklch::new(0.78, 0.17, 148.0),
        Oklch::new(0.50, 0.10, 25.0),
        Oklch::new(0.30, 0.02, 300.0),
    ] {
        assert!(
            original.is_in_srgb_gamut(),
            "the test's own fixture is out of gamut: {original:?}"
        );
        let (r, g, b) = original.to_srgb();
        let back = srgb_to_oklch(r, g, b);
        assert!(
            (back.l - original.l).abs() < 0.01,
            "lightness drifted: {original:?} → {back:?}"
        );
        assert!(
            (back.c - original.c).abs() < 0.01,
            "chroma drifted: {original:?} → {back:?}"
        );
        // Hue precision degrades as chroma approaches zero — the angle is
        // computed from two numbers that are both nearly nothing, and 8-bit
        // quantisation moves it several degrees. That is a property of the
        // colour space, not a fault in the conversion, so the tolerance
        // follows the chroma rather than being flat.
        let tolerance = (0.3 / original.c.max(0.001)).clamp(1.0, 6.0);
        assert!(
            hue_distance(back.h, original.h) < tolerance,
            "hue drifted more than {tolerance:.1}°: {original:?} → {back:?}"
        );
    }
}

#[test]
fn an_out_of_gamut_colour_loses_chroma_and_keeps_its_hue() {
    // The reason `to_srgb` reduces chroma instead of clamping channels.
    // Clamping an over-saturated orange pins its red at 255 first and slides
    // the result towards yellow — a different colour from the one asked for.
    let asked = Oklch::new(0.70, 0.36, 25.0);
    let (r, g, b) = asked.to_srgb();
    let got = srgb_to_oklch(r, g, b);
    assert!(got.c < asked.c, "an impossible chroma must be reduced");
    assert!(
        hue_distance(got.h, asked.h) < 4.0,
        "hue must survive gamut clipping: asked {}, got {}",
        asked.h,
        got.h
    );
    assert!(
        (got.l - asked.l).abs() < 0.03,
        "lightness must survive gamut clipping"
    );
}

#[test]
fn a_lightness_ramp_is_perceptually_even() {
    // The whole reason for authoring in Oklch. Five equal steps in Oklch
    // lightness must come back as five roughly equal steps — an sRGB ramp that
    // measures even does not.
    let base = Oklch::new(0.0, 0.05, 250.0);
    let measured: Vec<f32> = (0..=4)
        .map(|i| {
            let step = base.at_lightness(0.2 + i as f32 * 0.15);
            let (r, g, b) = step.to_srgb();
            srgb_to_oklch(r, g, b).l
        })
        .collect();
    for pair in measured.windows(2) {
        let delta = pair[1] - pair[0];
        assert!(
            (delta - 0.15).abs() < 0.02,
            "uneven step {delta} in {measured:?}"
        );
    }
}

#[test]
fn high_contrast_removes_every_translucency() {
    // A widget's visual language is translucency, and translucency is exactly
    // what fails for someone who asked the system for more contrast. If any of
    // these is below 1.0, the setting is decorative.
    let theme = Theme::default().high_contrast();
    for role in [
        Role::Display,
        Role::Title,
        Role::Section,
        Role::Body,
        Role::BodyStrong,
        Role::Caption,
        Role::Chip,
    ] {
        assert_eq!(theme.text(role).opacity, 1.0, "{role:?} is still faded");
    }
    for role in [Role::Panel, Role::Card, Role::Chip] {
        let surface = theme.surface(role).expect("a surface role");
        assert_eq!(surface.fill_top.3, 1.0, "{role:?} is still translucent");
        assert_eq!(surface.fill_bottom.3, 1.0, "{role:?} is still translucent");
    }
}

#[test]
fn high_contrast_keeps_the_muted_step_visible() {
    // Muting is how "this is secondary" is said. At high contrast it must
    // survive as a smaller difference rather than disappearing into the
    // background — a stale reading still has to read as stale.
    let standard = Theme::default();
    let high = standard.high_contrast();
    let alpha = |t: &Theme| t.foreground(Accent::Neutral, Role::Body, true).3;
    assert!(alpha(&high) > alpha(&standard));
    assert!(alpha(&high) < 1.0, "muted must still differ from unmuted");
}

#[test]
fn the_hierarchy_still_exists_when_opacity_cannot_carry_it() {
    // With every opacity forced to 1, size and weight are all that is left to
    // separate a title from a caption.
    let theme = Theme::default().high_contrast();
    let title = theme.text(Role::Title);
    let body = theme.text(Role::Body);
    let caption = theme.text(Role::Caption);
    assert!(title.size > body.size && body.size > caption.size);
    assert!(title.weight > body.weight);
}

#[test]
fn light_and_dark_get_different_colours_rather_than_one_compromise() {
    let dark = Theme::default();
    let light = Theme::light();
    for accent in [
        Accent::Neutral,
        Accent::Brand,
        Accent::Positive,
        Accent::Warning,
        Accent::Critical,
    ] {
        let (d, l) = (dark.accent(accent), light.accent(accent));
        assert!(
            (d.l - l.l).abs() > 0.05,
            "{accent:?} uses the same lightness in both appearances"
        );
    }
    // The foreground has to invert, or nothing is readable in one of them.
    assert!(dark.accent(Accent::Neutral).l > 0.8);
    assert!(light.accent(Accent::Neutral).l < 0.4);
}

#[test]
fn series_colours_stay_apart_including_where_the_list_would_have_repeated() {
    // A fixed palette of eight repeats on the ninth and puts two identical
    // colours side by side in a legend. Walking the hue circle by the golden
    // angle does not.
    let theme = Theme::default();
    let hues: Vec<f32> = (0..12).map(|i| theme.accent(Accent::Series(i)).h).collect();
    for (i, a) in hues.iter().enumerate() {
        for (j, b) in hues.iter().enumerate() {
            if i != j {
                assert!(
                    hue_distance(*a, *b) > 12.0,
                    "series {i} and {j} are {:.1}° apart",
                    hue_distance(*a, *b)
                );
            }
        }
    }
}

#[test]
fn series_indices_wrap_without_panicking() {
    // Any `u8` is a valid index by construction — a widget with more series
    // than the palette has should get repeats, not a crash.
    let theme = Theme::default();
    for i in [0u8, 1, 7, 8, 200, 255] {
        let colour = theme.accent(Accent::Series(i));
        assert!((0.0..360.0).contains(&colour.h));
    }
}

#[test]
fn spacing_is_a_multiple_of_one_step() {
    // `gap(2)` means two steps everywhere, which is what keeps rhythm
    // consistent across widgets that have never heard of each other.
    let theme = Theme::default();
    assert_eq!(theme.spacing(0), 0.0);
    assert_eq!(theme.spacing(1), theme.step);
    assert_eq!(theme.spacing(3), theme.step * 3.0);
}

#[test]
fn only_container_roles_paint_a_surface() {
    let theme = Theme::default();
    for role in [Role::Panel, Role::Card, Role::Chip] {
        assert!(theme.surface(role).is_some(), "{role:?} should paint");
    }
    for role in [Role::Plain, Role::Title, Role::Body, Role::Caption] {
        assert!(theme.surface(role).is_none(), "{role:?} should not paint");
    }
}

#[test]
fn only_the_panel_casts_a_shadow() {
    // A card inside a panel with its own drop shadow reads as floating twice,
    // which is the classic way a widget starts looking like a web page.
    let theme = Theme::default();
    assert!(theme.surface(Role::Panel).unwrap().shadow.0 > 0.0);
    assert_eq!(theme.surface(Role::Card).unwrap().shadow.0, 0.0);
    assert_eq!(theme.surface(Role::Chip).unwrap().shadow.0, 0.0);
}

#[test]
fn text_sizes_scale_with_the_base_rather_than_being_fixed() {
    // A user who enlarges the base size must get the whole hierarchy larger,
    // not a title that stays put while the body grows past it.
    let small = Theme::default();
    let large = Theme {
        base_font_size: 20.0,
        ..Theme::default()
    };
    let ratio = large.text(Role::Display).size / small.text(Role::Display).size;
    let expected = 20.0 / small.base_font_size;
    assert!((ratio - expected).abs() < 0.001);
    assert!(large.text(Role::Caption).size > small.text(Role::Body).size);
}

#[test]
fn every_colour_the_theme_can_produce_is_inside_the_srgb_gamut() {
    // A palette that relies on clipping is a palette whose author did not
    // choose what ships: the clip pulls chroma in by however much each entry
    // happens to need, so a set meant to look uniform does not. Several of
    // these were outside on the first attempt, including the default brand
    // colour and every series entry.
    let mut checked = 0;
    for appearance in [Appearance::Dark, Appearance::Light] {
        for contrast in [Contrast::Standard, Contrast::High] {
            let theme = Theme {
                appearance,
                contrast,
                ..Theme::default()
            };
            let mut accents = vec![
                Accent::Neutral,
                Accent::Brand,
                Accent::Positive,
                Accent::Warning,
                Accent::Critical,
            ];
            accents.extend((0u8..=32).map(Accent::Series));
            for accent in accents {
                let colour = theme.accent(accent);
                assert!(
                    colour.is_in_srgb_gamut(),
                    "{accent:?} is outside sRGB in {appearance:?}/{contrast:?}: {colour:?}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 100, "the sweep did not actually cover much");
}

#[test]
fn series_colours_are_all_equally_saturated() {
    // The reason the series chroma is the narrowest the hue circle allows
    // rather than the widest. If any entry had to be clipped, the palette
    // would have members that look washed out next to their neighbours.
    let theme = Theme::default();
    let measured: Vec<f32> = (0..12)
        .map(|i| {
            let (r, g, b) = theme.accent(Accent::Series(i)).to_srgb();
            srgb_to_oklch(r, g, b).c
        })
        .collect();
    let lowest = measured.iter().cloned().fold(f32::MAX, f32::min);
    let highest = measured.iter().cloned().fold(0.0f32, f32::max);
    assert!(
        highest - lowest < 0.01,
        "series saturation varies from {lowest:.3} to {highest:.3}: {measured:?}"
    );
}

#[test]
fn gamut_clipping_finds_the_boundary_rather_than_overshooting_it() {
    // Bisection, not repeated scaling. A fixed 8 % step can leave up to 8 % of
    // the available chroma unused, which across a palette is visible.
    let asked = Oklch::new(0.70, 0.40, 25.0);
    let (r, g, b) = asked.to_srgb();
    let got = srgb_to_oklch(r, g, b);
    // The true boundary at this lightness and hue is ~0.191.
    assert!(
        got.c > 0.185,
        "clipping left chroma on the table: got {:.4}",
        got.c
    );
    assert!(got.c < 0.196, "clipping produced an impossible chroma");
}
