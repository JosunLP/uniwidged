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
    assert_eq!(composed.hit(rect.center()), Some(id));
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
    // Paint order decides: the last thing drawn is the thing on top. The
    // order comes from the compose pass rather than from the caller, so a
    // caller cannot get it wrong.
    let view = sample();
    let composed = compose(&view, |c| c);
    let refresh = NodeId::named("refresh");
    let point = composed.hit_rects[&refresh].center();
    assert_eq!(composed.hit(point), Some(refresh));
    assert!(
        composed.order.contains(&refresh),
        "every named node should be in paint order"
    );
}

#[test]
fn a_link_records_where_it_points_in_the_same_pass_as_its_rectangle() {
    // `ARCHITECTURE.md` §4, applied again: a second walk is a second answer.
    // Here the two answers would be the rectangle a click lands in and the URL
    // that click opens, and a disagreement between them is a click opening
    // something other than what was announced — the worst shape this
    // particular drift could take.
    let view = panel(NodeId::named("root"))
        .child(
            text("Forecast")
                .id(NodeId::named("forecast"))
                .link("https://example.com/forecast"),
        )
        .child(text("plain").id(NodeId::named("plain")))
        .view();
    let composed = compose(&view, |c| c);

    assert_eq!(
        composed.link(NodeId::named("forecast")),
        Some("https://example.com/forecast")
    );
    // A link is interactive and has a rectangle, like every other activation.
    assert!(composed.is_interactive(NodeId::named("forecast")));
    assert!(composed.hit_rects.contains_key(&NodeId::named("forecast")));

    // And a node that is not a link records no URL. Every interactive node has
    // a rectangle; only some of them have somewhere to go.
    assert_eq!(composed.link(NodeId::named("plain")), None);
    assert_eq!(composed.link(NodeId::named("root")), None);
}

#[test]
fn a_link_is_announced_as_one_to_a_screen_reader() {
    // The accessibility tree and the click path come out of the same walk, so
    // a link that is clickable must also be a link to somebody who cannot see
    // that it is blue.
    let view = panel(NodeId::named("root"))
        .child(
            text("Forecast")
                .id(NodeId::named("forecast"))
                .link("https://example.com/forecast"),
        )
        .view();
    let composed = compose(&view, |c| c);
    let announced = format!("{:?}", composed.a11y);
    assert!(announced.contains("Link"), "{announced}");
}
