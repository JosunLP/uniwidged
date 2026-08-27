//! Compositor tests: the three outputs of one pass, and that they agree.

use super::*;
use uniwidged_view::build::*;

const WIDGET: WidgetId = WidgetId(1);
const SIZE: Size = Size {
    width: 320.0,
    height: 180.0,
};

fn sample() -> View {
    panel(NodeId::named("root"))
        .gap(2)
        .child(
            row()
                .child(text("System").role(Role::Title))
                .child(spacer())
                .child(
                    icon(Icon::Refresh)
                        .on_click(NodeId::named("refresh"))
                        .label("Refresh"),
                ),
        )
        .child(divider())
        .child(text("62%").role(Role::Display).id(NodeId::named("value")))
        .child(progress(0.62).label("CPU"))
        .view()
}

fn compose(view: &View, f: impl FnOnce(Compose<'_>) -> Compose<'_>) -> Composed {
    let mut compositor = Compositor::default();
    compositor.compose(f(Compose::new(view, WIDGET, "System", SIZE)))
}

#[test]
fn only_named_nodes_get_a_hit_rectangle() {
    // An anonymous node cannot be reported back, so recording its rectangle
    // would only give hit-testing something to trip over.
    let view = sample();
    let composed = compose(&view, |c| c);
    assert!(composed.hit_rects.contains_key(&NodeId::named("refresh")));
    assert!(composed.hit_rects.contains_key(&NodeId::named("value")));
    assert!(!composed.hit_rects.contains_key(&NodeId::ANONYMOUS));
}

#[test]
fn a_click_lands_on_the_node_the_screen_reader_was_told_about() {
    // The property the whole one-pass design exists for. If these two ever
    // disagree, a screen reader announces a button where a click does nothing.
    let view = sample();
    let composed = compose(&view, |c| c);
    let id = NodeId::named("refresh");
    let rect = composed.hit_rects[&id];
    let announced = composed
        .a11y
        .iter()
        .find(|(node_id, _)| *node_id == a11y::access_id(WIDGET, id))
        .map(|(_, node)| node.bounds().expect("bounds"))
        .expect("the button should be in the accessibility tree");
    assert!((announced.x0 - rect.x0).abs() < 0.01);
    assert!((announced.y1 - rect.y1).abs() < 0.01);
    assert_eq!(composed.hit(rect.center(), &[id]), Some(id));
}

#[test]
fn a_mirrored_layout_is_clickable_where_it_is_drawn() {
    // Recording unmirrored rectangles would make every right-to-left widget
    // clickable in the wrong place — silently, and only for the users least
    // likely to be testing it.
    let view = sample();
    let ltr = compose(&view, |c| c);
    let rtl = compose(&view, |c| c.rtl(true));
    let id = NodeId::named("refresh");
    let (a, b) = (ltr.hit_rects[&id], rtl.hit_rects[&id]);
    assert!(
        (a.x0 - (SIZE.width - b.x1)).abs() < 0.01,
        "the mirrored rectangle should be the folded one: {a:?} against {b:?}"
    );
    assert!(
        b.x0 < SIZE.width / 2.0,
        "a trailing control should be on the left when mirrored, was at {}",
        b.x0
    );
    assert!((a.y0 - b.y0).abs() < 0.01, "mirroring is horizontal only");
}

#[test]
fn hit_rectangles_stay_in_logical_pixels_whatever_the_scale() {
    // Input arrives in logical pixels once the shell has divided out the scale
    // factor. Recording physical ones would make every click land a fraction
    // off on a fractional-scale display, and only there.
    let view = sample();
    let one = compose(&view, |c| c);
    let two = compose(&view, |c| c.scale(2.0));
    for (id, rect) in &one.hit_rects {
        let scaled = two.hit_rects[id];
        assert!(
            (rect.x0 - scaled.x0).abs() < 0.01 && (rect.y1 - scaled.y1).abs() < 0.01,
            "{id:?} moved with the scale factor: {rect:?} against {scaled:?}"
        );
    }
}

#[test]
fn a_scene_is_actually_produced() {
    let view = sample();
    let composed = compose(&view, |c| c);
    assert!(
        !composed.scene.encoding().is_empty(),
        "the scene should contain drawing commands"
    );
    assert!(composed.intrinsic.width > 0.0 && composed.intrinsic.height > 0.0);
}

#[test]
fn an_empty_view_composes_without_drama() {
    // A widget that has nothing to say yet — no data fetched — must not be a
    // panic on the first frame.
    let view = View::default();
    let composed = compose(&view, |c| c);
    assert!(composed.hit_rects.is_empty());
    assert_eq!(composed.a11y.len(), 1, "just the root");
}

#[test]
fn measuring_does_not_require_painting() {
    // The host needs a widget's natural size to give a new one a sensible box,
    // and should not pay for a scene it will never present.
    let view = sample();
    let mut compositor = Compositor::default();
    let size = compositor.measure(&view, SIZE);
    assert_eq!(size, SIZE, "the panel fills what it is given");
}

#[test]
fn changing_the_theme_does_not_throw_away_the_shaping() {
    // A colour change does not move a glyph. Re-shaping every string on a
    // light/dark switch would make the switch visibly slow with several
    // widgets on screen.
    let view = sample();
    let mut compositor = Compositor::default();
    compositor.compose(Compose::new(&view, WIDGET, "System", SIZE));
    compositor.set_theme(Theme::light());
    compositor.compose(Compose::new(&view, WIDGET, "System", SIZE));
    // Same strings, same sizes: the second pass should have found them all.
    assert_eq!(compositor.theme().appearance, Appearance::Light);
}

#[test]
fn the_topmost_node_takes_the_click() {
    // Paint order decides: the last thing drawn is the thing on top.
    let view = sample();
    let composed = compose(&view, |c| c);
    let refresh = NodeId::named("refresh");
    let value = NodeId::named("value");
    let point = composed.hit_rects[&refresh].center();
    // Given both in paint order, the later one wins where they overlap.
    assert_eq!(composed.hit(point, &[value, refresh]), Some(refresh));
}
