//! Docking and undocking, without a monitor.
//!
//! Every test here is a hardware change: a screen appears, disappears, or moves
//! in the layout. The whole point of storing an output *name* is that these
//! stop mattering, and the only way to know they have is to write them down.

use super::*;

/// A laptop screen and an external monitor to the left of it — the layout that
/// breaks index-based placement, because plugging the external in renumbers
/// everything *and* moves the laptop's origin.
fn docked() -> Vec<Output> {
    vec![
        Output {
            name: "DP-1".into(),
            rect: PhysicalRect::new(0, 0, 2560, 1440),
            primary: false,
        },
        Output {
            name: "eDP-1".into(),
            rect: PhysicalRect::new(2560, 0, 1920, 1080),
            primary: true,
        },
    ]
}

fn undocked() -> Vec<Output> {
    vec![Output {
        name: "eDP-1".into(),
        rect: PhysicalRect::new(0, 0, 1920, 1080),
        primary: true,
    }]
}

fn placement(output: Option<&str>, x: i32, y: i32) -> Placement {
    Placement {
        output: output.map(str::to_owned),
        x,
        y,
        width: 320,
        height: 200,
        locked: false,
    }
}

#[test]
fn a_placement_on_a_named_output_is_relative_to_it() {
    // 48,48 on the laptop screen is 2608,48 on the desktop while docked. Stored
    // as the desktop coordinate it would land in the middle of the external
    // monitor the moment the layout changed.
    let resolved = resolve(&placement(Some("eDP-1"), 48, 48), &docked());
    assert_eq!(resolved.rect, PhysicalRect::new(2608, 48, 320, 200));
    assert_eq!(resolved.output.as_deref(), Some("eDP-1"));
    assert!(!resolved.displaced);
}

#[test]
fn the_same_placement_follows_its_monitor_when_the_layout_changes() {
    // Undocking moves the laptop's origin to 0,0. The widget is still 48,48 on
    // *its own screen*, which is where the user put it.
    let stored = placement(Some("eDP-1"), 48, 48);
    assert_eq!(
        resolve(&stored, &undocked()).rect,
        PhysicalRect::new(48, 48, 320, 200)
    );
    assert_eq!(
        resolve(&stored, &docked()).rect,
        PhysicalRect::new(2608, 48, 320, 200)
    );
}

#[test]
fn a_widget_whose_monitor_is_gone_moves_to_the_primary_and_remembers_where_it_belongs() {
    // The half that is easy to get wrong: the widget has to be visible *now*,
    // and it has to go home when the monitor comes back. Rewriting the stored
    // name would satisfy the first and quietly lose the second.
    let resolved = resolve(&placement(Some("DP-1"), 100, 100), &undocked());
    assert_eq!(resolved.rect, PhysicalRect::new(100, 100, 320, 200));
    assert!(resolved.displaced, "the caller should be able to say so");
    assert_eq!(
        resolved.output.as_deref(),
        Some("eDP-1"),
        "it is on the primary now"
    );
    // And nothing here has touched the stored placement, so plugging the
    // monitor back in puts it back.
    let stored = placement(Some("DP-1"), 100, 100);
    assert_eq!(
        resolve(&stored, &docked()).rect,
        PhysicalRect::new(100, 100, 320, 200)
    );
}

#[test]
fn a_widget_too_big_for_its_new_monitor_keeps_its_size() {
    // Undocking from a 4K screen must not shrink a widget: it would not grow
    // back, and nobody asked. It is moved so that what fits is visible.
    let stored = Placement {
        output: Some("DP-1".into()),
        x: 2000,
        y: 1200,
        width: 900,
        height: 600,
        locked: false,
    };
    let resolved = resolve(&stored, &undocked());
    assert_eq!(resolved.rect.width, 900);
    assert_eq!(resolved.rect.height, 600);
    assert_eq!(
        resolved.rect,
        PhysicalRect::new(1020, 480, 900, 600),
        "moved up and left until it fits, and no further"
    );
}

#[test]
fn a_widget_larger_than_the_whole_monitor_starts_at_its_corner() {
    let stored = Placement {
        output: Some("DP-1".into()),
        x: 40,
        y: 40,
        width: 2400,
        height: 1400,
        locked: false,
    };
    let resolved = resolve(&stored, &undocked());
    assert_eq!(
        resolved.rect,
        PhysicalRect::new(0, 0, 2400, 1400),
        "the top-left corner is the most of it anybody can see"
    );
}

#[test]
fn a_placement_with_no_output_is_a_desktop_coordinate() {
    // What `uniwidged install` writes, and what a hand-edited configuration
    // most naturally says. It is used as-is, and the output it happens to land
    // on is reported so the caller can record it.
    let resolved = resolve(&placement(None, 2700, 200), &docked());
    assert_eq!(resolved.rect, PhysicalRect::new(2700, 200, 320, 200));
    assert_eq!(resolved.output.as_deref(), Some("eDP-1"));
    assert!(!resolved.displaced);
}

#[test]
fn a_desktop_that_cannot_enumerate_its_monitors_still_places_widgets() {
    // An X server without RandR, or a platform whose shell does not answer
    // yet. Guessing a layout would be worse than using the coordinates as
    // given.
    let resolved = resolve(&placement(Some("DP-1"), 48, 48), &[]);
    assert_eq!(resolved.rect, PhysicalRect::new(48, 48, 320, 200));
    assert_eq!(resolved.output, None);
    assert!(!resolved.displaced);
}

#[test]
fn a_dragged_widget_is_recorded_against_the_monitor_showing_most_of_it() {
    // By the centre, not the origin. A widget straddling the seam belongs to
    // the screen it is mostly on, and one just off the left edge of the laptop
    // must not be attributed to the monitor next door.
    let outputs = docked();
    assert_eq!(
        locate(PhysicalRect::new(2608, 48, 320, 200), &outputs),
        (Some("eDP-1".into()), PhysicalRect::new(48, 48, 320, 200))
    );
    // Origin on DP-1, centre on eDP-1.
    assert_eq!(
        locate(PhysicalRect::new(2500, 48, 320, 200), &outputs).0,
        Some("eDP-1".into())
    );
    // Straddling the other way: origin still left of the seam, and now the
    // centre is too. (2400 would have put the centre at exactly 2560, which is
    // the first pixel of eDP-1 — the seam belongs to the screen on the right,
    // and this test asserted otherwise until it was run.)
    assert_eq!(
        locate(PhysicalRect::new(2360, 48, 320, 200), &outputs).0,
        Some("DP-1".into())
    );
    assert_eq!(
        locate(PhysicalRect::new(2400, 48, 320, 200), &outputs).0,
        Some("eDP-1".into()),
        "a centre of exactly 2560 is the first pixel of the right-hand screen"
    );
}

#[test]
fn a_widget_off_every_monitor_is_recorded_without_one() {
    // Possible on a desktop with a gap between monitors, or a widget dragged
    // into the dead space some layouts leave. Inventing a name would put it
    // somewhere else next time.
    let (name, rect) = locate(PhysicalRect::new(-4000, -4000, 320, 200), &docked());
    assert_eq!(name, None);
    assert_eq!(rect, PhysicalRect::new(-4000, -4000, 320, 200));
}

#[test]
fn placing_and_recording_are_inverses() {
    // The property that matters after a drag: whatever is stored, resolving it
    // has to give back the rectangle the widget is actually at. A sign error
    // in either direction survives every test above and breaks this one.
    let outputs = docked();
    for rect in [
        PhysicalRect::new(2608, 48, 320, 200),
        PhysicalRect::new(10, 10, 320, 200),
        PhysicalRect::new(2000, 1200, 320, 200),
    ] {
        let (output, relative) = locate(rect, &outputs);
        let stored = Placement {
            output,
            x: relative.x,
            y: relative.y,
            width: relative.width,
            height: relative.height,
            locked: false,
        };
        assert_eq!(resolve(&stored, &outputs).rect, rect, "{rect:?}");
    }
}
