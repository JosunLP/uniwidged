//! What the pointer means.
//!
//! Every test here is about an ambiguity: the same press can be a click, a
//! move or a resize, and getting the rule wrong produces a widget that either
//! cannot be moved or cannot be clicked. Both have been shipped by other
//! people, which is why the rule is written down in the module note and
//! asserted here rather than left to the reading of `press`.

use super::*;

const WIDGET: WidgetId = WidgetId(1);

/// 320×200 at 48,48 — the default placement a widget is installed with.
fn geometry() -> PhysicalRect {
    PhysicalRect::new(48, 48, 320, 200)
}

/// The middle of the widget, in its own coordinates: not on any edge.
const INSIDE: (i32, i32) = (160, 100);

fn interaction() -> Interaction {
    Interaction::new(1.0)
}

/// Presses at the middle of the widget, on nothing interactive.
fn press_inside(pointer: &mut Interaction) -> Press {
    pointer.press(WIDGET, (200, 148), INSIDE, geometry(), false, false)
}

#[test]
fn a_press_that_does_not_move_is_a_click() {
    // The reason a click is delivered on release: until the pointer has
    // travelled, there is no way to know which of the two this was.
    let mut pointer = interaction();
    assert_eq!(press_inside(&mut pointer), Press::Move);
    assert_eq!(pointer.motion((201, 148)), None, "one pixel is not a drag");
    assert!(!pointer.is_dragging());
    assert_eq!(
        pointer.release((201, 148)),
        Release::Click {
            widget: WIDGET,
            at: INSIDE
        },
        "a press that stayed put is a click, at the point it was aimed"
    );
}

#[test]
fn a_press_that_travels_moves_the_widget() {
    let mut pointer = interaction();
    press_inside(&mut pointer);
    // Past the threshold.
    let moved = pointer.motion((240, 178)).expect("this is a drag");
    assert!(pointer.is_dragging());
    assert_eq!(moved, PhysicalRect::new(88, 78, 320, 200));

    match pointer.release((240, 178)) {
        Release::Placed { widget, rect } => {
            assert_eq!(widget, WIDGET);
            assert_eq!(rect, PhysicalRect::new(88, 78, 320, 200));
        }
        other => panic!("a finished drag should place the widget, got {other:?}"),
    }
}

#[test]
fn every_step_of_a_drag_is_measured_from_where_it_started() {
    // Not from the previous step. Accumulating deltas makes a slow drag and a
    // fast one end in different places, because each step rounds — and the
    // difference only shows up on a machine slow enough to coalesce motion
    // events, which is never the machine it was written on.
    let mut pointer = interaction();
    press_inside(&mut pointer);
    for step in 1..=20 {
        let at = (200 + step, 148 + step);
        if let Some(rect) = pointer.motion(at) {
            assert_eq!(rect, PhysicalRect::new(48 + step, 48 + step, 320, 200));
        }
    }
    assert_eq!(
        pointer.motion((400, 348)).expect("still dragging"),
        PhysicalRect::new(248, 248, 320, 200),
        "a jump to the end lands where the arithmetic says, not where the \
         accumulated steps do"
    );
}

#[test]
fn a_press_on_something_interactive_belongs_to_the_widget() {
    // A button is the widget's, and a widget covered in buttons must still be
    // movable — which is why the rule is about the node under the pointer
    // rather than about the widget.
    let mut pointer = interaction();
    assert_eq!(
        pointer.press(WIDGET, (200, 148), INSIDE, geometry(), false, true),
        Press::ToWidget
    );
    assert_eq!(
        pointer.motion((260, 208)),
        None,
        "a button is not a handle, however far the pointer travels"
    );
    assert_eq!(
        pointer.release((260, 208)),
        Release::Nothing,
        "and a press dragged off a button is not a click either"
    );
}

#[test]
fn a_locked_widget_is_furniture() {
    // What the `locked` switch is for: everything works except moving it.
    // Clicks still reach the widget, which is the half people forget.
    let mut pointer = interaction();
    assert_eq!(
        pointer.press(WIDGET, (200, 148), INSIDE, geometry(), true, false),
        Press::ToWidget
    );
    assert_eq!(pointer.motion((400, 348)), None);
    assert_eq!(pointer.release((400, 348)), Release::Nothing);

    // And a press that does not move is still a click.
    let mut pointer = interaction();
    pointer.press(WIDGET, (200, 148), INSIDE, geometry(), true, false);
    assert!(matches!(pointer.release((200, 148)), Release::Click { .. }));
}

#[test]
fn a_locked_widget_cannot_be_resized_either() {
    let mut pointer = interaction();
    assert_eq!(
        pointer.press(WIDGET, (48, 48), (0, 0), geometry(), true, false),
        Press::ToWidget,
        "the corner of a locked widget is not a resize handle"
    );
}

#[test]
fn the_frame_resizes_and_the_corners_take_both_axes() {
    let size = (320, 200);
    let grip = 6.0;
    assert_eq!(Edge::at((0, 0), size, grip), Some(Edge::TopLeft));
    assert_eq!(Edge::at((319, 199), size, grip), Some(Edge::BottomRight));
    assert_eq!(Edge::at((319, 0), size, grip), Some(Edge::TopRight));
    assert_eq!(Edge::at((0, 199), size, grip), Some(Edge::BottomLeft));
    assert_eq!(Edge::at((160, 2), size, grip), Some(Edge::Top));
    assert_eq!(Edge::at((160, 198), size, grip), Some(Edge::Bottom));
    assert_eq!(Edge::at((2, 100), size, grip), Some(Edge::Left));
    assert_eq!(Edge::at((318, 100), size, grip), Some(Edge::Right));
    // Everything else is the body.
    assert_eq!(Edge::at((160, 100), size, grip), None);
    assert_eq!(Edge::at((7, 100), size, grip), None);
    // And a point outside the window is not an edge at all.
    assert_eq!(Edge::at((-1, 100), size, grip), None);
    assert_eq!(Edge::at((400, 100), size, grip), None);
}

#[test]
fn a_widget_narrower_than_two_grips_can_still_be_grabbed_from_both_sides() {
    // The minimum size is larger than this, but the arithmetic must not
    // depend on that: a grip wider than half the widget would make the left
    // edge win the whole width and the right edge unreachable.
    let size = (10, 200);
    assert_eq!(Edge::at((1, 100), size, 6.0), Some(Edge::Left));
    assert_eq!(Edge::at((9, 100), size, 6.0), Some(Edge::Right));
}

#[test]
fn the_frame_wins_over_a_button_on_the_edge() {
    // A control sitting on the very edge would otherwise make the widget
    // unresizable from that side, and there is nowhere else to grab.
    let mut pointer = interaction();
    assert_eq!(
        pointer.press(WIDGET, (367, 247), (319, 199), geometry(), false, true),
        Press::Resize(Edge::BottomRight)
    );
}

#[test]
fn dragging_the_bottom_right_corner_grows_the_widget_and_leaves_its_origin() {
    let mut pointer = interaction();
    pointer.press(WIDGET, (367, 247), (319, 199), geometry(), false, false);
    let grown = pointer.motion((467, 347)).expect("resizing");
    assert_eq!(grown, PhysicalRect::new(48, 48, 420, 300));
}

#[test]
fn dragging_the_top_left_corner_moves_the_origin_and_keeps_the_far_edge() {
    // The half that is easy to get wrong: growing upward means the *position*
    // changes as well as the size, and the bottom-right corner must not move
    // by a single pixel while it happens.
    let mut pointer = interaction();
    pointer.press(WIDGET, (48, 48), (0, 0), geometry(), false, false);
    let grown = pointer.motion((28, 18)).expect("resizing");
    assert_eq!(grown, PhysicalRect::new(28, 18, 340, 230));
    assert_eq!(
        (grown.x + grown.width as i32, grown.y + grown.height as i32),
        (368, 248),
        "the opposite corner stayed where it was"
    );
}

#[test]
fn a_widget_cannot_be_resized_inside_out() {
    // Dragging the left edge past the right one stops at the minimum width,
    // and *stopping* means the right edge does not move — the classic version
    // of this bug lets the box invert and the widget vanishes.
    let mut pointer = interaction();
    pointer.press(WIDGET, (48, 48), (0, 0), geometry(), false, false);
    let squashed = pointer.motion((900, 900)).expect("resizing");

    assert_eq!(squashed.width, MIN_SIZE.0);
    assert_eq!(squashed.height, MIN_SIZE.1);
    assert_eq!(
        (
            squashed.x + squashed.width as i32,
            squashed.y + squashed.height as i32
        ),
        (368, 248),
        "the far corner is where it was, so the widget is still where it was"
    );
}

#[test]
fn the_minimum_size_holds_from_every_edge() {
    for (edge, local, to) in [
        (Edge::Left, (0, 100), (900, 148)),
        (Edge::Right, (319, 100), (-900, 148)),
        (Edge::Top, (160, 0), (200, 900)),
        (Edge::Bottom, (160, 199), (200, -900)),
    ] {
        let mut pointer = interaction();
        pointer.press(WIDGET, (200, 148), local, geometry(), false, false);
        let rect = pointer.motion(to).expect("resizing");
        assert!(
            rect.width >= MIN_SIZE.0 && rect.height >= MIN_SIZE.1,
            "{edge:?} squashed the widget to {}x{}",
            rect.width,
            rect.height
        );
    }
}

#[test]
fn a_widget_cannot_be_dragged_out_of_reach() {
    // A widget with nothing left on screen is one the user cannot get back:
    // there is no titlebar to find it by and no task switcher it appears in.
    let mut pointer = interaction();
    pointer.set_bounds(Some(PhysicalRect::new(0, 0, 1280, 720)));
    press_inside(&mut pointer);

    let far_right = pointer.motion((9000, 148)).expect("dragging");
    assert!(
        far_right.x < 1280 && far_right.x + far_right.width as i32 > 1280 - 320,
        "something must remain on screen, got {far_right:?}"
    );
    assert!(
        (1280 - far_right.x) as f64 >= KEEP_ON_SCREEN,
        "at least the keep-on-screen margin, got {}",
        1280 - far_right.x
    );

    let far_left = pointer.motion((-9000, 148)).expect("dragging");
    assert!(
        far_left.x + far_left.width as i32 >= KEEP_ON_SCREEN as i32,
        "and the same on the other side, got {far_left:?}"
    );

    let far_up = pointer.motion((200, -9000)).expect("dragging");
    assert_eq!(far_up.y, 0, "never above the desktop's top edge");
}

#[test]
fn without_bounds_nothing_is_clamped() {
    // A headless host and a test both run with no idea where the screen is.
    // Inventing one would move widgets for reasons the caller cannot see.
    let mut pointer = interaction();
    press_inside(&mut pointer);
    let far = pointer.motion((9000, 9000)).expect("dragging");
    assert_eq!(far.x, 48 + 8800);
}

#[test]
fn the_threshold_is_in_logical_pixels() {
    // Four device pixels on a 200 % display is two logical ones, which is
    // inside anybody's hand tremor. The threshold has to scale or a
    // high-density screen turns clicks into drags.
    let mut pointer = Interaction::new(2.0);
    pointer.press(WIDGET, (200, 148), INSIDE, geometry(), false, false);
    assert_eq!(
        pointer.motion((205, 148)),
        None,
        "five device pixels at 200 % is two and a half logical ones"
    );
    assert!(pointer.motion((210, 148)).is_some());
}

#[test]
fn a_second_press_replaces_the_first() {
    // Only one thing can be dragged at a time. A stale gesture surviving a new
    // press is how a widget starts following the pointer after the button has
    // been released.
    let mut pointer = interaction();
    press_inside(&mut pointer);
    pointer.motion((260, 208));
    assert!(pointer.is_dragging());

    let other = WidgetId(2);
    pointer.press(other, (10, 10), (5, 5), geometry(), false, false);
    assert_eq!(pointer.widget(), Some(other));
    assert!(!pointer.is_dragging(), "the new press has not moved yet");
}

#[test]
fn a_release_with_nothing_held_is_harmless() {
    // The pointer can come up over a widget that never saw it go down —
    // a grab that was broken, a window that appeared under the pointer.
    let mut pointer = interaction();
    assert_eq!(pointer.release((10, 10)), Release::Nothing);
    assert_eq!(pointer.widget(), None);
}

#[test]
fn a_cancelled_gesture_gives_back_the_geometry_it_started_with() {
    let mut pointer = interaction();
    press_inside(&mut pointer);
    pointer.motion((400, 348));
    assert_eq!(pointer.cancel(), Some(geometry()));
    assert_eq!(pointer.widget(), None);
    assert_eq!(pointer.cancel(), None);
}
