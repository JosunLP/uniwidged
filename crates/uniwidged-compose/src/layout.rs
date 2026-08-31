//! Where everything goes.
//!
//! Turns a [`View`] into a tree of absolute rectangles, using [`boxes`] for the
//! box model — taffy, behind a vocabulary a widget can express — and
//! [`crate::text`] for the one thing taffy cannot know: how wide a string is.
//!
//! ## Sizes come from the theme, not from the widget
//!
//! A widget says `gap(2)`; this asks the theme what a step is. A widget says
//! [`Role::Title`]; this asks the theme how large that is and measures the
//! string at that size. Nothing here has a pixel constant in it that a theme
//! could not override, which is what makes a user-scaled base size actually
//! scale the layout rather than only the glyphs.
//!
//! ## Truncation is decided here and applied later
//!
//! A text leaf is measured at its natural width. If layout then gives it less,
//! the box records how much it got and [`crate::paint`] ellipsises to fit. The
//! alternative — deciding truncation during measurement — needs the final width
//! before layout has computed it, which is circular.

pub mod boxes;

use boxes::{Align as FAlign, Axis, Edges, Justify as FJustify, LayoutTree, Length, Style};
use uniwidged_core::{NodeId, Rect};
use uniwidged_view::{Align, Content, Direction, Justify, Node, Role, View};

use crate::text::{Shaping, TextEngine};
use crate::theme::Theme;

/// One node's place in the finished layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed<'a> {
    pub node: &'a Node,
    /// Absolute, in logical pixels from the surface's top-left corner.
    pub rect: Rect,
    /// The rectangle inside this node's padding, which is where its children
    /// and its own text live.
    pub content: Rect,
    pub children: Vec<Placed<'a>>,
}

impl<'a> Placed<'a> {
    /// Every node in the tree, parents before children — paint order.
    pub fn walk(&self, visit: &mut impl FnMut(&Placed<'a>)) {
        visit(self);
        for child in &self.children {
            child.walk(visit);
        }
    }

    /// The deepest node containing `point` whose id is not anonymous.
    ///
    /// Deepest wins: a button inside a row inside a panel should report the
    /// button. Anonymous nodes are skipped rather than blocking — decoration
    /// drawn over a row must not swallow the row's clicks.
    pub fn hit(&self, point: uniwidged_core::Point) -> Option<NodeId> {
        if !self.rect.contains(point) {
            return None;
        }
        for child in self.children.iter().rev() {
            if let Some(hit) = child.hit(point) {
                return Some(hit);
            }
        }
        (!self.node.id.is_anonymous()).then_some(self.node.id)
    }
}

/// A text leaf awaiting measurement, referenced from taffy by index.
struct TextLeaf {
    text: String,
    shaping: Shaping,
    wrap: bool,
}

/// Lays out a view tree inside `available` logical pixels.
pub fn compute<'a>(
    view: &'a View,
    theme: &Theme,
    text: &mut TextEngine,
    available: (f32, f32),
) -> Placed<'a> {
    compute_with_images(view, theme, text, available, None)
}

/// [`compute`], with the widget's images — which is what gives an `image`
/// node a size. Without them it has none.
pub fn compute_with_images<'a>(
    view: &'a View,
    theme: &Theme,
    text: &mut TextEngine,
    available: (f32, f32),
    images: Option<&crate::Images>,
) -> Placed<'a> {
    let mut tree = LayoutTree::new();
    let mut leaves: Vec<TextLeaf> = Vec::new();
    let mut context = Build {
        theme,
        engine: text,
        tree: &mut tree,
        leaves: &mut leaves,
        images,
    };
    let root = context.build(&view.root);

    tree.compute(root, available.0, available.1, |m| {
        // Key 0 means "not a measured leaf"; `boxes` never calls us with it —
        // it has a test for that — but the arithmetic below would silently
        // underflow if it ever did.
        let Some(leaf) = leaves.get(m.key as usize - 1) else {
            return (0.0, 0.0);
        };
        // Rounded up, always. Taffy rounds computed boxes to whole pixels, so
        // a natural width of 26.7 becomes a box of 26 — and the paint pass,
        // finding the text one third of a pixel too wide for its own box,
        // ellipsises it. "CPU" rendered as "C…" with half the panel empty
        // until this `ceil` was here.
        let (width, height) = if leaf.wrap {
            let width = m.available_width.unwrap_or(f32::INFINITY);
            let shaped = text.shape(
                &leaf.text,
                leaf.shaping,
                width.is_finite().then_some(width),
                crate::text::Align::Start,
            );
            (shaped.width(), shaped.height())
        } else {
            // Natural width. If layout gives it less, `paint` ellipsises —
            // see the module note on why that cannot be decided here.
            text.measure(&leaf.text, leaf.shaping)
        };
        (width.ceil(), height.ceil())
    });

    read_back(&view.root, root, &tree, theme, (0.0, 0.0))
}

/// Everything the tree-building pass carries down the tree.
struct Build<'t> {
    theme: &'t Theme,
    engine: &'t mut TextEngine,
    tree: &'t mut LayoutTree,
    leaves: &'t mut Vec<TextLeaf>,
    images: Option<&'t crate::Images>,
}

impl Build<'_> {
    fn build(&mut self, node: &Node) -> boxes::NodeId {
        let Build {
            theme,
            engine,
            tree,
            leaves,
            images,
        } = self;
        let theme: &Theme = theme;
        let style = box_style(node, theme);
        match &node.content {
            Content::Text(string) => {
                let shaping =
                    Shaping::from_style(theme.text(node.style.role), is_numeric(node.style.role));
                if node.style.role == Role::Chip {
                    // A chip is a pill around its text, so it has to hug it. An
                    // explicit width overrides the parent's stretch, exactly as in
                    // CSS — and measuring it here rather than through the measure
                    // callback is what makes the width explicit at all.
                    let (width, _) = engine.measure(string, shaping);
                    let (pad_x, _) = theme.chip_padding();
                    leaves.push(TextLeaf {
                        text: string.clone(),
                        shaping,
                        wrap: false,
                    });
                    return tree.measured_leaf(
                        // `shrink(0)` as well as an explicit width: without it a
                        // row that overflows makes taffy shrink whatever it can,
                        // and a chip narrower than its text is a chip reading
                        // "li…". Text *should* shrink — that is how it comes to be
                        // ellipsised — but a pill that hugs its content has
                        // nothing to give.
                        // The *whole box* is rounded up, not just the text.
                        // Taffy rounds a computed box down to whole pixels, and a
                        // chip's padding is a fraction of its font size — so
                        // ceiling only the text still leaves 32.44 becoming 32,
                        // and the text a third of a pixel too wide for its own
                        // content box. That rendered "live" as "li…", and only on
                        // a display whose font made the arithmetic land badly.
                        style
                            .width(Length::px((width + pad_x * 2.0).ceil()))
                            .shrink(0.0),
                        leaves.len() as u64,
                    );
                }
                leaves.push(TextLeaf {
                    text: string.clone(),
                    shaping,
                    wrap: node.style.layout.wrap,
                });
                tree.measured_leaf(style, leaves.len() as u64)
            }
            Content::Icon(_) => {
                // Square, and not shrinkable for the same reason as a chip: an
                // icon squeezed narrower than it is tall stops being the icon it
                // was drawn as.
                let size = theme.text(node.style.role).size * 1.15;
                tree.leaf(style.size(Length::px(size), Length::px(size)).shrink(0.0))
            }
            Content::Divider => {
                // Explicitly full width: the default cross-axis alignment sizes a
                // child to its content, and a rule with no content is invisible.
                let (_, thickness) = theme.divider();
                tree.leaf(style.width(Length::FILL).height(Length::px(thickness)))
            }
            Content::Progress(_) => tree.leaf(
                style
                    .width(Length::FILL)
                    .height(Length::px(theme.progress_thickness())),
            ),
            // Full width and a theme height, like a progress bar: the widget says
            // *what* the series is, and how much room a chart gets is the one
            // decision that has to be the same across every widget on the screen.
            Content::Sparkline(_) => tree.leaf(
                style
                    .width(Length::FILL)
                    .height(Length::px(theme.sparkline_height())),
            ),
            Content::Spacer => tree.leaf(style),
            // At its own size, in logical pixels — one image pixel per logical
            // pixel, so a 32-pixel icon is a 32-pixel icon on every display and
            // the scale factor makes it crisp rather than large. Not shrinkable:
            // an icon squeezed narrower than it is tall is a different picture.
            // A handle with no image behind it takes no room, so a widget whose
            // icon failed to register shows a layout with a gap rather than a
            // layout with a hole the size of a picture that is not there.
            Content::Image(handle) => match images.and_then(|images| images.get(handle)) {
                Some(image) => tree.leaf(
                    style
                        .size(
                            Length::px(image.width as f32),
                            Length::px(image.height as f32),
                        )
                        .shrink(0.0),
                ),
                None => tree.leaf(style.size(Length::px(0.0), Length::px(0.0))),
            },
            Content::Group => {
                let children: Vec<_> = node.children.iter().map(|c| self.build(c)).collect();
                self.tree.container(style, &children)
            }
        }
    }
}

/// Should this role's digits be tabular?
///
/// [`Role::Display`] is the large numeric readout — a clock, a temperature, a
/// percentage — and it is the one that visibly twitches without `tnum`.
/// [`Role::Chip`] is usually a count, and a chip that changes width as its
/// number ticks moves everything after it.
///
/// Takes a role rather than a node so that [`crate::Measurer`] can ask the
/// same question. Tabular figures change a string's width, so a widget
/// measuring one thing and being laid out as another would truncate against
/// the wrong number.
pub(crate) fn is_numeric(role: Role) -> bool {
    matches!(role, Role::Display | Role::Chip)
}

fn box_style(node: &Node, theme: &Theme) -> Style {
    let layout = &node.style.layout;
    let mut style = Style::default()
        .direction(match layout.direction {
            Direction::Row => Axis::Horizontal,
            Direction::Column => Axis::Vertical,
        })
        .gap(theme.spacing(layout.gap))
        .align(match layout.align {
            Align::Start => FAlign::Start,
            Align::Center => FAlign::Center,
            Align::End => FAlign::End,
            Align::Stretch => FAlign::Stretch,
        })
        .justify(match layout.justify {
            Justify::Start => FJustify::Start,
            Justify::Center => FJustify::Center,
            Justify::End => FJustify::End,
            Justify::SpaceBetween => FJustify::SpaceBetween,
        })
        .grow(layout.grow as f32);

    // A chip pads itself. Its author asked for a pill, not for a box they then
    // have to space by hand — and the padding has to be proportional to the
    // text or the pill stops being a pill when the base size changes.
    let padding = if node.style.role == Role::Chip {
        let (h, v) = theme.chip_padding();
        Edges::symmetric(h, v)
    } else {
        Edges::all(theme.spacing(layout.padding))
    };
    style = style.padding(padding);

    // The panel is the whole surface, so it fills it. Everything else is sized
    // by its content and by what its parent gives it.
    if node.style.role == Role::Panel {
        style = style.size(Length::FILL, Length::FILL);
    }
    style
}

fn read_back<'a>(
    node: &'a Node,
    taffy: boxes::NodeId,
    tree: &LayoutTree,
    theme: &Theme,
    origin: (f64, f64),
) -> Placed<'a> {
    let box_of = tree.box_of(taffy);
    let x = origin.0 + box_of.x;
    let y = origin.1 + box_of.y;
    let rect = Rect::new(x, y, x + box_of.width, y + box_of.height);

    let (pad_x, pad_y) = if node.style.role == Role::Chip {
        theme.chip_padding()
    } else {
        let p = theme.spacing(node.style.layout.padding);
        (p, p)
    };
    let (pad_x, pad_y) = (pad_x as f64, pad_y as f64);
    // Clamped so that a box smaller than its own padding yields an empty
    // content rectangle rather than an inverted one, which every downstream
    // `contains` would then answer wrongly.
    let content = Rect::new(
        rect.x0 + pad_x,
        rect.y0 + pad_y,
        (rect.x1 - pad_x).max(rect.x0 + pad_x),
        (rect.y1 - pad_y).max(rect.y0 + pad_y),
    );

    let children = if matches!(node.content, Content::Group) {
        let taffy_children = tree.children(taffy);
        node.children
            .iter()
            .zip(taffy_children)
            .map(|(child, child_taffy)| read_back(child, child_taffy, tree, theme, (x, y)))
            .collect()
    } else {
        Vec::new()
    };

    Placed {
        node,
        rect,
        content,
        children,
    }
}

#[cfg(test)]
mod tests;
