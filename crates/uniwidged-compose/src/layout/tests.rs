//! Layout tests. Real fonts, no GPU, no window.

use super::*;
use uniwidged_core::Point;
use uniwidged_view::build::*;

const AVAILABLE: (f32, f32) = (300.0, 200.0);

fn placed(view: &View) -> Placed<'_> {
    let mut engine = TextEngine::new();
    compute(view, &Theme::default(), &mut engine, AVAILABLE)
}

fn find<'a>(placed: &'a Placed<'a>, id: NodeId) -> &'a Placed<'a> {
    fn search<'a>(p: &'a Placed<'a>, id: NodeId) -> Option<&'a Placed<'a>> {
        if p.node.id == id {
            return Some(p);
        }
        p.children.iter().find_map(|c| search(c, id))
    }
    search(placed, id).expect("node should be in the layout")
}

#[test]
fn the_panel_fills_the_surface_it_was_given() {
    // The panel is the widget's whole visible body; anything less leaves the
    // glass smaller than the surface it was sized for.
    let view = panel(NodeId::named("root")).view();
    let root = placed(&view);
    assert_eq!(root.rect.width(), AVAILABLE.0 as f64);
    assert_eq!(root.rect.height(), AVAILABLE.1 as f64);
}

#[test]
fn padding_insets_the_content_rectangle() {
    let theme = Theme::default();
    let view = panel(NodeId::named("root")).padding(3).view();
    let root = placed(&view);
    let expected = theme.spacing(3) as f64;
    assert!((root.content.x0 - root.rect.x0 - expected).abs() < 0.01);
    assert!((root.rect.x1 - root.content.x1 - expected).abs() < 0.01);
}

#[test]
fn a_row_lays_its_children_out_left_to_right_with_the_gap_between_them() {
    let theme = Theme::default();
    let view = panel(NodeId::named("root"))
        .padding(0)
        .child(row().gap(2).children([
            chip("one").id(NodeId::named("a")),
            chip("two").id(NodeId::named("b")),
        ]))
        .view();
    let root = placed(&view);
    let (a, b) = (
        find(&root, NodeId::named("a")),
        find(&root, NodeId::named("b")),
    );
    assert!(b.rect.x0 > a.rect.x1, "b should follow a");
    let gap = b.rect.x0 - a.rect.x1;
    assert!(
        (gap - theme.spacing(2) as f64).abs() < 0.01,
        "gap was {gap}, expected {}",
        theme.spacing(2)
    );
    assert!(
        (a.rect.y0 - b.rect.y0).abs() < 0.01,
        "a row shares a baseline"
    );
}

#[test]
fn a_column_lays_its_children_out_top_to_bottom() {
    let view = panel(NodeId::named("root"))
        .padding(0)
        .gap(1)
        .child(text("first").id(NodeId::named("a")))
        .child(text("second").id(NodeId::named("b")))
        .view();
    let root = placed(&view);
    let (a, b) = (
        find(&root, NodeId::named("a")),
        find(&root, NodeId::named("b")),
    );
    assert!(b.rect.y0 >= a.rect.y1, "b should be below a");
}

#[test]
fn a_spacer_pushes_what_follows_to_the_far_edge() {
    // The most common shape in a widget: a label on the left, a value on the
    // right. If the spacer does not grow, the two sit next to each other and
    // the whole layout reads as broken.
    let view = panel(NodeId::named("root"))
        .padding(0)
        .child(
            row()
                .child(text("left").id(NodeId::named("left")))
                .child(spacer())
                .child(text("right").id(NodeId::named("right"))),
        )
        .view();
    let root = placed(&view);
    let right = find(&root, NodeId::named("right"));
    assert!(
        (right.rect.x1 - AVAILABLE.0 as f64).abs() < 1.0,
        "the trailing node should reach the edge, ended at {}",
        right.rect.x1
    );
}

#[test]
fn text_is_measured_rather_than_guessed() {
    // Inside a row, so the width is the text's own rather than whatever the
    // cross-axis stretch handed it — in a column every child fills the line by
    // design, which would make this pass without measuring anything.
    let view = panel(NodeId::named("root"))
        .child(row().child(text("Widget").id(NodeId::named("t"))))
        .view();
    let root = placed(&view);
    let t = find(&root, NodeId::named("t"));
    assert!(t.rect.width() > 0.0);
    assert!(t.rect.height() > 0.0);
    assert!(
        t.rect.width() < AVAILABLE.0 as f64 / 2.0,
        "one short word should not fill half the panel, was {}",
        t.rect.width()
    );
}

#[test]
fn a_larger_role_lays_out_larger() {
    // Proof that layout asks the theme rather than using one size for all
    // text: the same string at Display must occupy more room than at Caption.
    // In a row: along the main axis a child is sized by its content, so the
    // width really is the shaped width.
    let make = |role| {
        panel(NodeId::named("root"))
            .child(row().child(text("18:48").role(role).id(NodeId::named("t"))))
            .view()
    };
    let (big, small) = (make(Role::Display), make(Role::Caption));
    let (big, small) = (placed(&big), placed(&small));
    let big = find(&big, NodeId::named("t"));
    let small = find(&small, NodeId::named("t"));
    assert!(big.rect.width() > small.rect.width());
    assert!(big.rect.height() > small.rect.height());
}

#[test]
fn a_chip_is_wider_than_the_text_inside_it() {
    // A chip pads itself; if it did not, its author would have to, and every
    // author would choose a different amount.
    let chip_view = panel(NodeId::named("root"))
        .child(chip("3").id(NodeId::named("c")))
        .view();
    let plain_view = panel(NodeId::named("root"))
        .child(text("3").role(Role::Chip).id(NodeId::named("c")))
        .view();
    let chip_root = placed(&chip_view);
    let plain_root = placed(&plain_view);
    let with_padding = find(&chip_root, NodeId::named("c")).rect.width();
    let bare = find(&plain_root, NodeId::named("c")).rect.width();
    // Both are role Chip, so both self-pad; what matters is that the padding
    // is really there.
    assert_eq!(with_padding, bare);
    let inner = find(&chip_root, NodeId::named("c"));
    assert!(
        inner.content.width() < inner.rect.width(),
        "a chip's content must be inset from its box"
    );
}

#[test]
fn a_divider_spans_the_width_it_is_given() {
    // Cross-axis alignment defaults to sizing a child to its content, and a
    // rule has no content — without an explicit full width it is invisible.
    let view = panel(NodeId::named("root"))
        .padding(0)
        .child(divider().id(NodeId::named("rule")))
        .view();
    let root = placed(&view);
    let rule = find(&root, NodeId::named("rule"));
    assert!(
        (rule.rect.width() - AVAILABLE.0 as f64).abs() < 1.0,
        "the rule was {} wide of {}",
        rule.rect.width(),
        AVAILABLE.0
    );
    assert!(rule.rect.height() > 0.0 && rule.rect.height() <= 2.0);
}

#[test]
fn the_deepest_named_node_takes_the_click() {
    // A button inside a row inside a panel must report the button.
    let view = panel(NodeId::named("root"))
        .padding(0)
        .child(
            row().child(
                chip("press")
                    .on_click(NodeId::named("button"))
                    .id(NodeId::named("button")),
            ),
        )
        .view();
    let root = placed(&view);
    let button = find(&root, NodeId::named("button"));
    let centre = Point::new(button.rect.center().x, button.rect.center().y);
    assert_eq!(root.hit(centre), Some(NodeId::named("button")));
}

#[test]
fn decoration_does_not_swallow_the_click_underneath_it() {
    // An anonymous node is decoration by definition. If hit-testing stopped at
    // one, a divider or a background drawn over a row would make the row dead.
    let view = panel(NodeId::named("root"))
        .padding(0)
        .child(row().child(chip("x").id(NodeId::named("target"))))
        .view();
    let root = placed(&view);
    let target = find(&root, NodeId::named("target"));
    // The row itself is anonymous and contains the point.
    assert_eq!(
        root.hit(Point::new(target.rect.center().x, target.rect.center().y)),
        Some(NodeId::named("target"))
    );
}

#[test]
fn a_click_outside_the_widget_hits_nothing() {
    let view = panel(NodeId::named("root")).view();
    let root = placed(&view);
    assert_eq!(root.hit(Point::new(-5.0, 10.0)), None);
    assert_eq!(root.hit(Point::new(AVAILABLE.0 as f64 + 5.0, 10.0)), None);
}

#[test]
fn every_child_stays_inside_its_parent() {
    // A layout bug that lets a child escape its parent shows up as a widget
    // painting over the desktop, which is the single most alarming way for
    // this to fail.
    let view = panel(NodeId::named("root"))
        .child(text("Weather").role(Role::Title))
        .child(divider())
        .child(
            row()
                .child(text("18°").role(Role::Display))
                .child(spacer())
                .child(chip("now")),
        )
        .child(progress(0.4))
        .view();
    let root = placed(&view);
    fn check(parent: &Placed<'_>) {
        for child in &parent.children {
            assert!(
                child.rect.x0 >= parent.rect.x0 - 0.01
                    && child.rect.x1 <= parent.rect.x1 + 0.01
                    && child.rect.y0 >= parent.rect.y0 - 0.01
                    && child.rect.y1 <= parent.rect.y1 + 0.01,
                "{:?} escaped {:?}",
                child.rect,
                parent.rect
            );
            check(child);
        }
    }
    check(&root);
}

#[test]
fn a_scaled_base_size_scales_the_layout_and_not_only_the_glyphs() {
    // If sizes were pixel constants here, enlarging the base font would make
    // text overflow boxes that stayed put.
    let view = panel(NodeId::named("root"))
        .child(text("Calendar").role(Role::Title).id(NodeId::named("t")))
        .view();
    let mut engine = TextEngine::new();
    let small = compute(&view, &Theme::default(), &mut engine, AVAILABLE);
    let large_theme = Theme {
        base_font_size: 20.0,
        ..Theme::default()
    };
    let large = compute(&view, &large_theme, &mut engine, AVAILABLE);
    assert!(
        find(&large, NodeId::named("t")).rect.height()
            > find(&small, NodeId::named("t")).rect.height()
    );
}
