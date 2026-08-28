//! Building a view tree without writing a struct literal.
//!
//! A widget rebuilds its whole tree on every change, so this is the code a
//! widget author writes most, and it should read like the layout it describes:
//!
//! ```
//! use uniwidged_view::build::*;
//! use uniwidged_core::NodeId;
//!
//! let view = panel(NodeId::named("clock"))
//!     .padding(2)
//!     .child(text("14:32").role(Role::Display))
//!     .child(text("Tuesday, 27 August").role(Role::Caption).muted())
//!     .view();
//! ```
//!
//! Every method takes and returns `self`, so a tree is one expression and the
//! nesting in the source is the nesting on screen.

use crate::{Direction, Ratio, Semantics, Style};
use uniwidged_core::NodeId;

// Re-exported rather than merely imported, so that
// `use uniwidged_view::build::*` is the only import a widget file needs: a
// builder chain reads badly with a module path in the middle of it.
pub use crate::{
    Accent, Align, Content, Icon, ImageHandle, Interaction, Justify, Node, Role, View,
};

impl Node {
    /// Wraps this node as the root of a view.
    pub fn view(self) -> View {
        View { root: self }
    }

    pub fn id(mut self, id: NodeId) -> Self {
        self.id = id;
        self
    }

    pub fn child(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }

    /// Appends many children — the shape a list takes, where the count is not
    /// known until the data arrives.
    pub fn children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children.extend(children);
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.style.role = role;
        self
    }

    pub fn accent(mut self, accent: Accent) -> Self {
        self.style.accent = accent;
        self
    }

    pub fn muted(mut self) -> Self {
        self.style.muted = true;
        self
    }

    pub fn row(mut self) -> Self {
        self.style.layout.direction = Direction::Row;
        self
    }

    pub fn column(mut self) -> Self {
        self.style.layout.direction = Direction::Column;
        self
    }

    /// Space between children, in theme steps — not pixels. See
    /// [`Layout`](crate::Layout).
    pub fn gap(mut self, steps: u8) -> Self {
        self.style.layout.gap = steps;
        self
    }

    pub fn padding(mut self, steps: u8) -> Self {
        self.style.layout.padding = steps;
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.style.layout.align = align;
        self
    }

    pub fn justify(mut self, justify: Justify) -> Self {
        self.style.layout.justify = justify;
        self
    }

    pub fn grow(mut self, share: u8) -> Self {
        self.style.layout.grow = share;
        self
    }

    pub fn wrap(mut self) -> Self {
        self.style.layout.wrap = true;
        self
    }

    /// Makes this node clickable. Requires a named id — an anonymous node
    /// cannot be reported back, so this would silently do nothing.
    pub fn on_click(mut self, id: NodeId) -> Self {
        debug_assert!(
            !id.is_anonymous(),
            "an interactive node needs a name: the click is reported by id"
        );
        self.id = id;
        self.semantics.interaction = Interaction::Button;
        self
    }

    pub fn link(mut self, url: impl Into<String>) -> Self {
        self.semantics.interaction = Interaction::Link(url.into());
        self
    }

    /// What a screen reader reads instead of this node's own text.
    ///
    /// Worth setting wherever the visible text is an abbreviation, a glyph or
    /// a number without its unit — "14:32" reads better as "half past two".
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.semantics.label = Some(label.into());
        self
    }

    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.semantics.tooltip = Some(text.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.semantics.state.disabled = disabled;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.semantics.state.selected = selected;
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.semantics.state.checked = Some(checked);
        self.semantics.interaction = Interaction::Toggle;
        self
    }
}

fn node(content: Content) -> Node {
    Node {
        id: NodeId::ANONYMOUS,
        content,
        style: Style::default(),
        semantics: Semantics::default(),
        children: Vec::new(),
    }
}

/// The widget's outermost surface. One per widget.
pub fn panel(id: NodeId) -> Node {
    node(Content::Group).id(id).role(Role::Panel).padding(2)
}

/// A container with no surface of its own.
pub fn group() -> Node {
    node(Content::Group)
}

/// A horizontal container. The most common shape in a widget: a label on the
/// left, a value on the right, a [`spacer`] between them.
pub fn row() -> Node {
    node(Content::Group).row().align(Align::Center)
}

/// A vertical container.
pub fn column() -> Node {
    node(Content::Group).column()
}

/// A raised surface inside the panel.
pub fn card() -> Node {
    node(Content::Group).role(Role::Card).padding(2)
}

pub fn text(s: impl Into<String>) -> Node {
    node(Content::Text(s.into()))
}

pub fn icon(icon: Icon) -> Node {
    node(Content::Icon(icon))
}

pub fn image(handle: ImageHandle) -> Node {
    node(Content::Image(handle))
}

pub fn divider() -> Node {
    node(Content::Divider)
}

/// Flexible empty space — the standard way to push what follows to the far
/// edge.
pub fn spacer() -> Node {
    node(Content::Spacer).grow(1)
}

pub fn progress(value: f32) -> Node {
    node(Content::Progress(Ratio::new(value)))
}

/// A small pill: a count, a tag, a status.
pub fn chip(s: impl Into<String>) -> Node {
    node(Content::Text(s.into())).role(Role::Chip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_reads_like_the_layout_it_describes() {
        let view = panel(NodeId::named("weather"))
            .child(text("Weather").role(Role::Title))
            .child(
                row()
                    .gap(1)
                    .child(text("18°").role(Role::Display))
                    .child(spacer())
                    .child(icon(Icon::Refresh).on_click(NodeId::named("refresh"))),
            )
            .view();

        assert_eq!(view.root.style.role, Role::Panel);
        assert_eq!(view.root.children.len(), 2);
        let temperature_row = &view.root.children[1];
        assert_eq!(temperature_row.style.layout.direction, Direction::Row);
        assert_eq!(temperature_row.children.len(), 3);
        assert_eq!(
            temperature_row.children[2].semantics.interaction,
            Interaction::Button
        );
    }

    #[test]
    fn a_spacer_pushes_what_follows_to_the_far_edge() {
        // Without `grow` a spacer is a zero-width node and the layout silently
        // does nothing, which is a maddening bug to look at.
        assert_eq!(spacer().style.layout.grow, 1);
    }

    #[test]
    fn state_setters_do_not_disturb_the_rest_of_the_node() {
        let n = text("Done").checked(true).muted().disabled(true);
        assert_eq!(n.semantics.state.checked, Some(true));
        assert_eq!(n.semantics.interaction, Interaction::Toggle);
        assert!(n.semantics.state.disabled);
        assert!(n.style.muted);
        assert_eq!(n.content, Content::Text("Done".into()));
    }

    #[test]
    #[should_panic(expected = "an interactive node needs a name")]
    fn an_anonymous_node_cannot_be_made_clickable() {
        // The click is reported by id, so an anonymous interactive node would
        // be a button that does nothing and says nothing about why.
        let _ = text("Refresh").on_click(NodeId::ANONYMOUS);
    }
}
