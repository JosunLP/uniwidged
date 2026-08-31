//! Accessibility tests. No GPU, no screen reader, no window.

use super::*;
// `Node` names two different things here: AccessKit's and the view tree's.
use crate::layout;
use crate::text::TextEngine;
use crate::theme::Theme;
use accesskit::Node as AccessNode;
use uniwidged_core::NodeId;
use uniwidged_view::View;
use uniwidged_view::build::*;

const WIDGET: WidgetId = WidgetId(7);

fn tree(view: &View) -> Vec<(AccessId, AccessNode)> {
    let mut engine = TextEngine::new();
    let placed = layout::compute(view, &Theme::default(), &mut engine, (300.0, 200.0));
    build(&placed, WIDGET, "System load")
}

fn node(nodes: &[(AccessId, AccessNode)], id: AccessId) -> &AccessNode {
    &nodes
        .iter()
        .find(|(n, _)| *n == id)
        .expect("node present")
        .1
}

#[test]
fn the_root_comes_first_and_names_the_widget() {
    // The host attaches the widget to the desktop's tree by this id, and a
    // screen reader announces the widget by this label.
    let view = panel(NodeId::named("root")).child(text("Idle")).view();
    let nodes = tree(&view);
    assert_eq!(nodes[0].0, root_id(WIDGET));
    assert_eq!(nodes[0].1.role(), AccessRole::Window);
    assert_eq!(nodes[0].1.label(), Some("System load"));
}

#[test]
fn scaffolding_is_not_read_aloud() {
    // A screen reader working through "group, group, separator, group" is
    // describing the layout rather than the content.
    let view = panel(NodeId::named("root"))
        .child(row().child(spacer()).child(divider()))
        .child(text("Actual content"))
        .view();
    let nodes = tree(&view);
    // The root plus exactly one label.
    assert_eq!(
        nodes.len(),
        2,
        "{:?}",
        nodes.iter().map(|(_, n)| n.role()).collect::<Vec<_>>()
    );
    assert_eq!(nodes[1].1.role(), AccessRole::Label);
}

#[test]
fn unnamed_content_is_still_read() {
    // An author names a node so that clicks can be reported back. A plain
    // label needs no name — and dropping it because it has none would silence
    // the widget's actual content.
    let view = panel(NodeId::named("root"))
        .child(text("Tuesday, 27 August"))
        .view();
    let nodes = tree(&view);
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[1].1.label(), Some("Tuesday, 27 August"));
}

#[test]
fn a_button_advertises_that_it_can_be_pressed() {
    let view = panel(NodeId::named("root"))
        .child(
            icon(Icon::Refresh)
                .on_click(NodeId::named("refresh"))
                .label("Refresh now"),
        )
        .view();
    let nodes = tree(&view);
    let button = node(&nodes, access_id(WIDGET, NodeId::named("refresh")));
    assert_eq!(button.role(), AccessRole::Button);
    assert_eq!(button.label(), Some("Refresh now"));
    assert!(button.supports_action(Action::Click));
}

#[test]
fn a_disabled_control_does_not_invite_a_press() {
    // Offering `Click` on a disabled control invites a screen reader user to
    // press something that will do nothing.
    let view = panel(NodeId::named("root"))
        .child(
            icon(Icon::Refresh)
                .on_click(NodeId::named("refresh"))
                .label("Refresh now")
                .disabled(true),
        )
        .view();
    let nodes = tree(&view);
    let button = node(&nodes, access_id(WIDGET, NodeId::named("refresh")));
    assert!(button.is_disabled());
    assert!(!button.supports_action(Action::Click));
}

#[test]
fn disabled_is_inherited_by_everything_inside() {
    // A button inside a disabled card is not clickable, and announcing it as
    // available would be a lie a user acts on.
    let view = panel(NodeId::named("root"))
        .child(
            card().disabled(true).child(
                chip("press")
                    .on_click(NodeId::named("inner"))
                    .label("Press me"),
            ),
        )
        .view();
    let nodes = tree(&view);
    let inner = node(&nodes, access_id(WIDGET, NodeId::named("inner")));
    assert!(
        inner.is_disabled(),
        "the ancestor's disabled state must reach it"
    );
    assert!(!inner.supports_action(Action::Click));
}

#[test]
fn a_toggle_reports_which_way_it_is_set() {
    let view = panel(NodeId::named("root"))
        .child(
            chip("Done")
                .id(NodeId::named("task"))
                .label("Task complete")
                .checked(true),
        )
        .view();
    let nodes = tree(&view);
    let toggle = node(&nodes, access_id(WIDGET, NodeId::named("task")));
    assert_eq!(toggle.role(), AccessRole::CheckBox);
    assert_eq!(toggle.toggled(), Some(Toggled::True));
}

#[test]
fn a_progress_bar_reports_a_percentage_rather_than_a_fraction() {
    // "0.62" is not what anybody wants read out.
    let view = panel(NodeId::named("root"))
        .child(progress(0.62).id(NodeId::named("cpu")).label("CPU"))
        .view();
    let nodes = tree(&view);
    let bar = node(&nodes, access_id(WIDGET, NodeId::named("cpu")));
    assert_eq!(bar.role(), AccessRole::ProgressIndicator);
    assert!((bar.numeric_value().unwrap_or_default() - 62.0).abs() < 1.0);
    assert_eq!(bar.min_numeric_value(), Some(0.0));
    assert_eq!(bar.max_numeric_value(), Some(100.0));
}

#[test]
fn a_chart_is_an_image_whose_value_is_its_latest_reading() {
    // A line has no sensible reading aloud; its latest point does. The label
    // says what is charted, the numeric value says where it is now, and the
    // same percentage convention as a progress bar applies.
    let view = panel(NodeId::named("root"))
        .child(
            sparkline([0.1, 0.4, 0.62])
                .id(NodeId::named("history"))
                .label("CPU over the last minute"),
        )
        .view();
    let nodes = tree(&view);
    let chart = node(&nodes, access_id(WIDGET, NodeId::named("history")));
    assert_eq!(chart.role(), AccessRole::Image);
    assert_eq!(chart.label(), Some("CPU over the last minute"));
    assert!((chart.numeric_value().unwrap_or_default() - 62.0).abs() < 1.0);
}

#[test]
fn an_unlabelled_chart_is_decoration_and_an_empty_one_has_no_value() {
    // Without a label a screen reader could only say "image", which is worse
    // than silence. And a chart with no points yet has no latest reading to
    // report — a zero would be a claim about the machine.
    let view = panel(NodeId::named("root"))
        .child(sparkline([0.5]).id(NodeId::named("mute")))
        .child(
            sparkline(std::iter::empty())
                .id(NodeId::named("empty"))
                .label("Nothing yet"),
        )
        .view();
    let nodes = tree(&view);
    assert!(
        !nodes
            .iter()
            .any(|(id, _)| *id == access_id(WIDGET, NodeId::named("mute"))),
        "an unlabelled chart should not be in the tree"
    );
    let empty = node(&nodes, access_id(WIDGET, NodeId::named("empty")));
    assert_eq!(empty.numeric_value(), None);
}

#[test]
fn a_tooltip_becomes_a_description() {
    let view = panel(NodeId::named("root"))
        .child(
            chip("stale")
                .id(NodeId::named("s"))
                .label("Stale")
                .tooltip("The last read failed"),
        )
        .view();
    let nodes = tree(&view);
    let chip = node(&nodes, access_id(WIDGET, NodeId::named("s")));
    assert_eq!(chip.description(), Some("The last read failed"));
}

#[test]
fn bounds_are_the_same_rectangles_the_layout_produced() {
    // The whole reason all three outputs come from one pass. A screen reader
    // whose bounds disagree with the hit rectangles puts the focus ring
    // somewhere the click does not go.
    let view = panel(NodeId::named("root"))
        .child(chip("x").on_click(NodeId::named("b")).label("Button"))
        .view();
    let mut engine = TextEngine::new();
    let placed = layout::compute(&view, &Theme::default(), &mut engine, (300.0, 200.0));
    let nodes = build(&placed, WIDGET, "w");

    fn find<'a>(p: &'a layout::Placed<'a>, id: NodeId) -> &'a layout::Placed<'a> {
        fn search<'a>(p: &'a layout::Placed<'a>, id: NodeId) -> Option<&'a layout::Placed<'a>> {
            if p.node.id == id {
                return Some(p);
            }
            p.children.iter().find_map(|c| search(c, id))
        }
        search(p, id).expect("in the layout")
    }
    let laid_out = find(&placed, NodeId::named("b")).rect;
    let announced = node(&nodes, access_id(WIDGET, NodeId::named("b")))
        .bounds()
        .expect("bounds");
    assert!((announced.x0 - laid_out.x0).abs() < 0.01);
    assert!((announced.y0 - laid_out.y0).abs() < 0.01);
    assert!((announced.x1 - laid_out.x1).abs() < 0.01);
    assert!((announced.y1 - laid_out.y1).abs() < 0.01);
}

#[test]
fn an_id_survives_a_change_elsewhere_in_the_tree() {
    // If the id of the row a screen reader is on changed when the row above it
    // updated, focus would jump — which is why ids come from the author and
    // never from a traversal index.
    let make = |first: &str| {
        panel(NodeId::named("root"))
            .child(text(first))
            .child(chip("go").on_click(NodeId::named("action")).label("Go"))
            .view()
    };
    let before = tree(&make("11:58"));
    let after = tree(&make("11:59"));
    let id = access_id(WIDGET, NodeId::named("action"));
    assert!(before.iter().any(|(n, _)| *n == id));
    assert!(after.iter().any(|(n, _)| *n == id));
}

#[test]
fn two_instances_of_the_same_widget_do_not_collide() {
    // Two clocks side by side share every author-chosen node id, and would
    // share every AccessKit id too without the instance in the high bits.
    let a = access_id(WidgetId(1), NodeId::named("clock"));
    let b = access_id(WidgetId(2), NodeId::named("clock"));
    assert_ne!(a, b);
    assert_ne!(root_id(WidgetId(1)), root_id(WidgetId(2)));
}

#[test]
fn a_synthetic_id_can_never_land_on_an_author_chosen_one() {
    // `NodeId::named` hashes across the whole 32-bit space, so without a
    // separate flag bit a synthetic id could steal a real node's focus.
    let widget = WidgetId(3);
    for index in [0usize, 1, 255, 65_536] {
        let synthetic = synthetic_id(widget, index);
        for name in ["a", "root", "refresh", "row-0", "task-17"] {
            assert_ne!(synthetic, access_id(widget, NodeId::named(name)));
        }
        assert_ne!(synthetic, root_id(widget));
    }
}

#[test]
fn every_child_the_root_names_actually_exists() {
    // AccessKit rejects a tree that references a node it was not given, and
    // the failure mode is a panic inside the platform adapter rather than
    // anything a caller can catch.
    let view = panel(NodeId::named("root"))
        .child(text("Weather"))
        .child(divider())
        .child(row().child(text("18°")).child(spacer()).child(chip("now")))
        .child(progress(0.4).label("Humidity"))
        .view();
    let nodes = tree(&view);
    let ids: Vec<_> = nodes.iter().map(|(id, _)| *id).collect();
    for child in nodes[0].1.children() {
        assert!(
            ids.contains(child),
            "the root names a missing child {child:?}"
        );
    }
    assert_eq!(
        nodes[0].1.children().len(),
        nodes.len() - 1,
        "every node but the root should be a child of it"
    );
}
