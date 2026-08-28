//! The box model, over [`taffy`].
//!
//! A small vocabulary — [`Length`], [`Edges`], [`Axis`], [`Align`],
//! [`Justify`], [`Style`] — and a tree that computes it. Text leaves are
//! measured through a caller-supplied closure, so this module knows nothing
//! about Parley and taffy knows nothing about fonts.
//!
//! ## Why the vocabulary is ours
//!
//! Taffy's own types are CSS's, and CSS has more of them than a widget can
//! express: a widget names a direction, a gap and an alignment, and there is
//! no `position: absolute`, no `z-index` and no `float` anywhere in
//! `uniwidged-view`. Translating at this boundary keeps the impossible cases
//! impossible rather than merely unused, and it means the layout engine is
//! replaceable without anything above this file noticing.
//!
//! ## Where it comes from
//!
//! Adapted from `ferrum-layout` in [Ferrum UI](https://github.com/j-pfalzgraf/ferrumui)
//! (MIT OR Apache-2.0), by the same author — see `NOTICE.md`. Trimmed to what
//! a widget can describe: grid containers, scroll containers, margins and the
//! three space-distribution modes `uniwidged-view` has no word for are gone,
//! because a facade with more surface than its only caller uses is one nobody
//! can safely change.

use taffy::prelude::*;

/// Taffy's opaque node handle. Callers treat it as a token.
pub use taffy::NodeId;

/// A length along one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    /// Sized by content and by flex rules.
    Auto,
    /// A fixed number of logical pixels.
    Px(f32),
    /// A fraction (0.0–1.0) of the parent's corresponding dimension.
    Fraction(f32),
}

impl Length {
    /// Fill the parent's corresponding dimension.
    pub const FILL: Length = Length::Fraction(1.0);

    pub fn px(value: f32) -> Self {
        Length::Px(value)
    }

    fn to_dimension(self) -> Dimension {
        match self {
            Length::Auto => Dimension::auto(),
            Length::Px(value) => Dimension::length(value),
            Length::Fraction(fraction) => Dimension::percent(fraction),
        }
    }
}

/// Padding, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edges {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Edges {
    pub const ZERO: Edges = Edges {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    /// The same inset on all four sides.
    pub fn all(value: f32) -> Self {
        Edges {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }

    /// Horizontal and vertical insets, which is what a chip's pill needs.
    pub fn symmetric(horizontal: f32, vertical: f32) -> Self {
        Edges {
            left: horizontal,
            right: horizontal,
            top: vertical,
            bottom: vertical,
        }
    }

    fn to_rect(self) -> Rect<LengthPercentage> {
        Rect {
            left: LengthPercentage::length(self.left),
            right: LengthPercentage::length(self.right),
            top: LengthPercentage::length(self.top),
            bottom: LengthPercentage::length(self.bottom),
        }
    }
}

/// Main-axis direction of a container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Axis {
    Horizontal,
    #[default]
    Vertical,
}

/// Cross-axis alignment of children — CSS `align-items`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    Start,
    Center,
    End,
    #[default]
    Stretch,
}

/// Main-axis distribution of children — CSS `justify-content`.
///
/// Four, not six: these are exactly the ones `uniwidged_view::Justify` can
/// name, and a mode a widget cannot ask for is one nothing can test.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
}

/// One box's layout description.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub direction: Axis,
    pub width: Length,
    pub height: Length,
    pub padding: Edges,
    pub gap: f32,
    pub align: Align,
    pub justify: Justify,
    /// How much of the leftover main-axis space this box claims, relative to
    /// its siblings.
    pub grow: f32,
    /// How readily it gives space back. `0.0` keeps a box at its natural size,
    /// which is what a chip and an icon need — see the callers, where both
    /// stop being themselves when squeezed.
    pub shrink: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            direction: Axis::Vertical,
            width: Length::Auto,
            height: Length::Auto,
            padding: Edges::ZERO,
            gap: 0.0,
            align: Align::Stretch,
            justify: Justify::Start,
            grow: 0.0,
            shrink: 1.0,
        }
    }
}

impl Style {
    pub fn direction(mut self, axis: Axis) -> Self {
        self.direction = axis;
        self
    }

    pub fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    pub fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }

    pub fn size(mut self, width: Length, height: Length) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn padding(mut self, padding: Edges) -> Self {
        self.padding = padding;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn justify(mut self, justify: Justify) -> Self {
        self.justify = justify;
        self
    }

    pub fn grow(mut self, grow: f32) -> Self {
        self.grow = grow;
        self
    }

    pub fn shrink(mut self, shrink: f32) -> Self {
        self.shrink = shrink;
        self
    }

    fn to_taffy(self) -> taffy::Style {
        taffy::Style {
            display: Display::Flex,
            flex_direction: match self.direction {
                Axis::Horizontal => FlexDirection::Row,
                Axis::Vertical => FlexDirection::Column,
            },
            size: Size {
                width: self.width.to_dimension(),
                height: self.height.to_dimension(),
            },
            padding: self.padding.to_rect(),
            gap: Size {
                width: LengthPercentage::length(self.gap),
                height: LengthPercentage::length(self.gap),
            },
            align_items: Some(match self.align {
                Align::Start => AlignItems::START,
                Align::Center => AlignItems::CENTER,
                Align::End => AlignItems::END,
                Align::Stretch => AlignItems::STRETCH,
            }),
            justify_content: Some(match self.justify {
                Justify::Start => JustifyContent::START,
                Justify::Center => JustifyContent::CENTER,
                Justify::End => JustifyContent::END,
                Justify::SpaceBetween => JustifyContent::SPACE_BETWEEN,
            }),
            flex_grow: self.grow,
            flex_shrink: self.shrink,
            ..Default::default()
        }
    }
}

/// A computed box: position relative to its parent's content box, and size.
///
/// `f64` to match kurbo, which is what everything downstream draws with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxLayout {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What the measurement closure is told about a leaf.
#[derive(Clone, Copy, Debug)]
pub struct Measure {
    /// A dimension layout has already fixed, if any.
    pub known_width: Option<f32>,
    pub known_height: Option<f32>,
    /// The width the content may occupy before it should wrap, if bounded.
    pub available_width: Option<f32>,
    /// The key the leaf was registered with. Never zero for a measured leaf —
    /// see [`LayoutTree::compute`].
    pub key: u64,
}

/// A tree of boxes, computed together.
pub struct LayoutTree {
    taffy: TaffyTree<u64>,
}

impl Default for LayoutTree {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutTree {
    pub fn new() -> Self {
        LayoutTree {
            taffy: TaffyTree::new(),
        }
    }

    /// A leaf that does not measure itself — a rule, a spacer, an icon.
    pub fn leaf(&mut self, style: Style) -> NodeId {
        self.taffy
            .new_leaf(style.to_taffy())
            .expect("taffy refused a leaf")
    }

    /// A leaf whose size comes from the measurement closure. `key` is handed
    /// back to it, and must not be zero.
    pub fn measured_leaf(&mut self, style: Style, key: u64) -> NodeId {
        debug_assert!(key != 0, "zero means `not a measured leaf`");
        self.taffy
            .new_leaf_with_context(style.to_taffy(), key)
            .expect("taffy refused a measured leaf")
    }

    pub fn container(&mut self, style: Style, children: &[NodeId]) -> NodeId {
        self.taffy
            .new_with_children(style.to_taffy(), children)
            .expect("taffy refused a container")
    }

    /// Computes the whole tree inside `width`×`height` logical pixels.
    ///
    /// `measure` is called once per measured leaf, possibly more than once for
    /// the same leaf: flex resolution may ask again with a different available
    /// width, which is why the callers cache their shaping.
    pub fn compute(
        &mut self,
        root: NodeId,
        width: f32,
        height: f32,
        mut measure: impl FnMut(Measure) -> (f32, f32),
    ) {
        let available = Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::Definite(height),
        };
        self.taffy
            .compute_layout_with_measure(root, available, |known, avail, _node, context, _style| {
                // No context means an unmeasured leaf, which contributes
                // nothing of its own.
                let key = context.copied().unwrap_or(0);
                if key == 0 {
                    return Size::ZERO;
                }
                let available_width = match avail.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                };
                let (measured_width, measured_height) = measure(Measure {
                    known_width: known.width,
                    known_height: known.height,
                    available_width,
                    key,
                });
                Size {
                    // A dimension layout has already fixed wins: the closure
                    // is being asked what it would like, not told what it gets.
                    width: known.width.unwrap_or(measured_width),
                    height: known.height.unwrap_or(measured_height),
                }
            })
            .expect("taffy could not compute a layout");
    }

    /// The children of a node, in order, so the computed tree can be walked
    /// alongside the view tree it came from.
    pub fn children(&self, node: NodeId) -> Vec<NodeId> {
        self.taffy.children(node).unwrap_or_default()
    }

    /// The computed box of a node, relative to its parent's content box.
    pub fn box_of(&self, node: NodeId) -> BoxLayout {
        let computed = self
            .taffy
            .layout(node)
            .expect("node has no computed layout");
        BoxLayout {
            x: computed.location.x as f64,
            y: computed.location.y as f64,
            width: computed.size.width as f64,
            height: computed.size.height as f64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_column_stacks_its_children_with_the_gap_between_them() {
        let mut tree = LayoutTree::new();
        let first = tree.leaf(Style::default().size(Length::px(100.0), Length::px(40.0)));
        let second = tree.leaf(Style::default().size(Length::px(100.0), Length::px(40.0)));
        let root = tree.container(
            Style::default()
                .direction(Axis::Vertical)
                .gap(10.0)
                .padding(Edges::all(8.0)),
            &[first, second],
        );
        tree.compute(root, 200.0, 200.0, |_| (0.0, 0.0));

        let first = tree.box_of(first);
        let second = tree.box_of(second);
        // The first child sits at the padding origin.
        assert_eq!((first.x, first.y), (8.0, 8.0));
        // The second is below it plus the gap: 8 + 40 + 10.
        assert_eq!((second.x, second.y), (8.0, 58.0));
    }

    #[test]
    fn a_measured_leaf_is_sized_by_the_closure_and_told_its_key() {
        let mut tree = LayoutTree::new();
        let leaf = tree.measured_leaf(Style::default(), 42);
        let root = tree.container(Style::default(), &[leaf]);
        tree.compute(root, 500.0, 500.0, |m| {
            assert_eq!(m.key, 42);
            (123.0, 17.0)
        });
        let leaf = tree.box_of(leaf);
        assert_eq!((leaf.width, leaf.height), (123.0, 17.0));
    }

    #[test]
    fn an_unmeasured_leaf_never_reaches_the_closure() {
        // The zero key is the sentinel the callers rely on: they index a
        // vector with `key - 1`, and being called with zero would underflow.
        let mut tree = LayoutTree::new();
        let plain = tree.leaf(Style::default());
        let root = tree.container(Style::default(), &[plain]);
        tree.compute(root, 100.0, 100.0, |m| {
            panic!(
                "an unmeasured leaf asked to be measured, with key {}",
                m.key
            )
        });
        assert_eq!(tree.box_of(plain).height, 0.0);
    }

    #[test]
    fn a_box_that_may_not_shrink_keeps_its_width_in_a_row_that_overflows() {
        // What a chip and an icon depend on. Without `shrink(0)` taffy takes
        // the space back from whichever child will give it, and a pill
        // narrower than its own text reads as truncated.
        let mut tree = LayoutTree::new();
        let fixed = tree.leaf(
            Style::default()
                .size(Length::px(80.0), Length::px(20.0))
                .shrink(0.0),
        );
        let flexible = tree.leaf(Style::default().size(Length::px(80.0), Length::px(20.0)));
        let root = tree.container(
            // Sized, not `Auto`: an auto-width row is as wide as its content,
            // so there is nothing to overflow and nothing to shrink. The panel
            // this stands in for fills its surface, which is what creates the
            // constraint.
            Style::default()
                .direction(Axis::Horizontal)
                .width(Length::px(80.0)),
            &[fixed, flexible],
        );
        // Half the width the two children want.
        tree.compute(root, 80.0, 40.0, |_| (0.0, 0.0));

        assert_eq!(tree.box_of(fixed).width, 80.0);
        assert!(
            tree.box_of(flexible).width < 80.0,
            "the shrinkable child should have given the space up"
        );
    }

    #[test]
    fn grow_shares_out_what_is_left_over() {
        let mut tree = LayoutTree::new();
        let fixed = tree.leaf(Style::default().size(Length::px(40.0), Length::px(20.0)));
        let greedy = tree.leaf(Style::default().height(Length::px(20.0)).grow(1.0));
        let root = tree.container(
            Style::default()
                .direction(Axis::Horizontal)
                .width(Length::px(200.0)),
            &[fixed, greedy],
        );
        tree.compute(root, 200.0, 40.0, |_| (0.0, 0.0));

        assert_eq!(tree.box_of(fixed).width, 40.0);
        assert_eq!(tree.box_of(greedy).width, 160.0);
    }

    #[test]
    fn fill_is_the_whole_of_the_parents_content_box() {
        let mut tree = LayoutTree::new();
        let child = tree.leaf(Style::default().size(Length::FILL, Length::FILL));
        let root = tree.container(
            Style::default()
                .size(Length::px(200.0), Length::px(100.0))
                .padding(Edges::all(12.0)),
            &[child],
        );
        tree.compute(root, 200.0, 100.0, |_| (0.0, 0.0));

        let child = tree.box_of(child);
        assert_eq!((child.x, child.y), (12.0, 12.0));
        assert_eq!((child.width, child.height), (176.0, 76.0));
    }

    #[test]
    fn fill_inside_an_unsized_parent_is_nothing_at_all() {
        // A percentage of `Auto` is zero, which is CSS's rule and taffy's, and
        // it is the reason `Role::Panel` is given an explicit `FILL`×`FILL`
        // rather than inheriting one: a panel that did not ask would collapse,
        // and every widget on the desktop with it.
        let mut tree = LayoutTree::new();
        let child = tree.leaf(Style::default().size(Length::FILL, Length::FILL));
        let root = tree.container(Style::default(), &[child]);
        tree.compute(root, 200.0, 100.0, |_| (0.0, 0.0));

        assert_eq!(tree.box_of(child).width, 0.0);
    }
}
