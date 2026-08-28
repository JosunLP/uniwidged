//! What a screen reader is told.
//!
//! Built in the same pass as the scene and the hit rectangles, from the same
//! placed tree, so the three cannot disagree. A screen reader announcing a
//! button where a click does nothing is worse than no screen reader support,
//! and that is exactly what three separate passes eventually produce.
//!
//! ## The tree is flattened
//!
//! Only nodes that mean something reach AccessKit: anything with a label, a
//! string, an interaction or a value. Anonymous rows, spacers and dividers are
//! layout, and a screen reader that reads them aloud is describing the
//! scaffolding rather than the content.
//!
//! What comes out is one root and a flat list of children in reading order.
//! For a panel the size of a widget that is the right shape — the alternative
//! is a nest of unnamed groups that a user has to descend through to reach two
//! lines of text. A widget large enough to want real grouping would want
//! `Role::Group` nodes, and that is a change to make when something needs it.
//!
//! ## Ids are stable, because focus depends on it
//!
//! An AccessKit id is derived from the widget instance and the author's own
//! [`NodeId`](uniwidged_core::NodeId), never from a traversal index. If the
//! id of the row a screen
//! reader is on changed when the row above it updated, focus would jump — and
//! a traversal index changes exactly that way.

use accesskit::{
    Action, Node, NodeId as AccessId, Rect as AccessRect, Role as AccessRole, Toggled,
};
use uniwidged_core::WidgetId;
use uniwidged_view::{Content, Interaction};

use crate::layout::Placed;

/// The AccessKit id for one node of one widget instance.
///
/// Three fields, packed: the widget instance in the high 31 bits, a flag in
/// bit 32, and the author's node id in the low 32. Two clocks side by side
/// cannot collide, and neither can reach the other's nodes.
///
/// The flag separates author-chosen ids from the synthetic ones given to
/// unnamed content — see `synthetic_id`. Without it the two share a 32-bit
/// space, and [`uniwidged_core::NodeId::named`] hashes across all of it, so a
/// synthetic id could land on a real one and steal a screen reader's focus.
pub fn access_id(widget: WidgetId, node: uniwidged_core::NodeId) -> AccessId {
    AccessId((widget.0 << 33) | node.0 as u64)
}

/// An id for a meaningful node the author never named.
///
/// Stable only as long as the tree's shape is, which is acceptable precisely
/// because nothing can focus or activate these — they are read, not addressed.
fn synthetic_id(widget: WidgetId, index: usize) -> AccessId {
    AccessId((widget.0 << 33) | (1 << 32) | index as u64)
}

/// The id of a widget's root node, which is also the id the host uses to attach
/// the widget to the desktop's accessibility tree.
pub fn root_id(widget: WidgetId) -> AccessId {
    access_id(widget, uniwidged_core::NodeId::ANONYMOUS)
}

/// Builds the flat node list for one widget.
///
/// The first entry is always the root, and every other entry is one of its
/// children in reading order.
pub fn build(root: &Placed<'_>, widget: WidgetId, label: &str) -> Vec<(AccessId, Node)> {
    let mut children = Vec::new();
    let mut nodes = Vec::new();
    collect(root, widget, &mut children, &mut nodes, false);

    let mut root_node = Node::new(AccessRole::Window);
    root_node.set_label(label.to_string());
    root_node.set_bounds(bounds(root));
    root_node.set_children(children);

    let mut out = vec![(root_id(widget), root_node)];
    out.extend(nodes);
    out
}

fn bounds(placed: &Placed<'_>) -> AccessRect {
    AccessRect {
        x0: placed.rect.x0,
        y0: placed.rect.y0,
        x1: placed.rect.x1,
        y1: placed.rect.y1,
    }
}

fn collect(
    placed: &Placed<'_>,
    widget: WidgetId,
    children: &mut Vec<AccessId>,
    nodes: &mut Vec<(AccessId, Node)>,
    ancestor_disabled: bool,
) {
    let node = placed.node;
    // Disabled is inherited: a button inside a disabled card is not clickable,
    // and announcing it as available would be a lie a user acts on.
    let disabled = ancestor_disabled || node.semantics.state.disabled;

    if let Some(built) = describe(placed, disabled) {
        // A meaningful node the author never named is still read aloud — a
        // plain label needs no id because nothing addresses it, but dropping
        // it would silence the widget's actual content.
        let id = if node.id.is_anonymous() {
            synthetic_id(widget, nodes.len())
        } else {
            access_id(widget, node.id)
        };
        children.push(id);
        nodes.push((id, built));
    }

    for child in &placed.children {
        collect(child, widget, children, nodes, disabled);
    }
}

/// The AccessKit node for one placed node, or `None` where it is scaffolding.
fn describe(placed: &Placed<'_>, disabled: bool) -> Option<Node> {
    let node = placed.node;
    let semantics = &node.semantics;

    let role = match (&semantics.interaction, &node.content) {
        (Interaction::Button, _) => AccessRole::Button,
        (Interaction::Toggle, _) => AccessRole::CheckBox,
        (Interaction::ListItem, _) => AccessRole::ListItem,
        (Interaction::Link(_), _) => AccessRole::Link,
        (Interaction::None, Content::Text(_)) => AccessRole::Label,
        (Interaction::None, Content::Progress(_)) => AccessRole::ProgressIndicator,
        (Interaction::None, Content::Image(_)) => AccessRole::Image,
        // Scaffolding. A screen reader reading out "group, group, separator"
        // is describing the layout rather than the content.
        (Interaction::None, _) => return None,
    };

    let label = semantics.label.clone().or_else(|| match &node.content {
        Content::Text(text) => Some(text.clone()),
        _ => None,
    });
    // An interactive node with no label at all is a button that announces
    // itself as "button" and nothing else. Better to say so here than to ship
    // it: the builder's `debug_assert` catches it in a debug build, and this
    // keeps it out of the tree rather than putting a useless entry in it.
    if label.is_none() && !matches!(node.content, Content::Progress(_)) {
        return None;
    }

    let mut built = Node::new(role);
    if let Some(label) = label {
        built.set_label(label);
    }
    if let Some(description) = &semantics.tooltip {
        built.set_description(description.clone());
    }
    built.set_bounds(bounds(placed));

    if disabled {
        built.set_disabled();
    } else if !matches!(semantics.interaction, Interaction::None) {
        // Only an enabled control advertises the action. Offering `Click` on a
        // disabled one invites a screen reader user to press something that
        // will do nothing.
        built.add_action(Action::Click);
    }

    if let Some(checked) = semantics.state.checked {
        built.set_toggled(if checked {
            Toggled::True
        } else {
            Toggled::False
        });
    }
    if semantics.state.selected {
        built.set_selected(true);
    }
    if let Content::Progress(ratio) = &node.content {
        // As a percentage, because that is what a screen reader reads out and
        // "0.62" is not what anybody wants to hear.
        built.set_numeric_value((ratio.get() * 100.0) as f64);
        built.set_min_numeric_value(0.0);
        built.set_max_numeric_value(100.0);
    }
    Some(built)
}

#[cfg(test)]
mod tests;
