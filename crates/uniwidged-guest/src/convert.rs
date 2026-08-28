//! View tree to wire form, on the guest side.

// The records and variants are defined in the `types` interface; the guest
// interface only references them.
use crate::Event;
use crate::bindings::uniwidged::widget::types as wit;
use uniwidged_core::{NodeId, Wake};
use uniwidged_view::View;

pub fn to_wire(view: &View) -> wit::Tree {
    let flat = view.flatten();
    wit::Tree {
        nodes: flat.nodes.into_iter().map(node).collect(),
    }
}

fn node(n: uniwidged_view::wire::WireNode) -> wit::Node {
    wit::Node {
        id: n.id,
        content: content(n.content),
        style: style(n.style),
        semantics: semantics(n.semantics),
        children: n.children,
    }
}

fn content(c: uniwidged_view::Content) -> wit::Content {
    use uniwidged_view::Content as C;
    match c {
        C::Group => wit::Content::Group,
        C::Text(t) => wit::Content::Text(t),
        C::Icon(i) => wit::Content::Icon(icon(i)),
        C::Image(h) => wit::Content::Image(h.0),
        C::Divider => wit::Content::Divider,
        C::Spacer => wit::Content::Spacer,
        C::Progress(r) => wit::Content::Progress(r.get()),
    }
}

fn icon(i: uniwidged_view::Icon) -> wit::Icon {
    use uniwidged_view::Icon as I;
    match i {
        I::Refresh => wit::Icon::Refresh,
        I::Settings => wit::Icon::Settings,
        I::Close => wit::Icon::Close,
        I::Chevron => wit::Icon::Chevron,
        I::Check => wit::Icon::Check,
        I::Alert => wit::Icon::Alert,
        I::Offline => wit::Icon::Offline,
    }
}

/// One place, because `measure-text` names a role too — and a role that
/// crossed the boundary as one thing in a view and another in a measurement
/// would be a widget measuring what it is not going to be drawn as.
pub(crate) fn role(role: uniwidged_view::Role) -> wit::Role {
    use uniwidged_view::Role;
    match role {
        Role::Plain => wit::Role::Plain,
        Role::Panel => wit::Role::Panel,
        Role::Card => wit::Role::Card,
        Role::Chip => wit::Role::Chip,
        Role::Title => wit::Role::Title,
        Role::Display => wit::Role::Display,
        Role::Section => wit::Role::Section,
        Role::Body => wit::Role::Body,
        Role::BodyStrong => wit::Role::BodyStrong,
        Role::Caption => wit::Role::Caption,
    }
}

fn style(s: uniwidged_view::Style) -> wit::Style {
    use uniwidged_view::{Accent, Align, Direction, Justify};
    wit::Style {
        role: role(s.role),
        accent: match s.accent {
            Accent::Neutral => wit::Accent::Neutral,
            Accent::Brand => wit::Accent::Brand,
            Accent::Positive => wit::Accent::Positive,
            Accent::Warning => wit::Accent::Warning,
            Accent::Critical => wit::Accent::Critical,
            Accent::Series(i) => wit::Accent::Series(i),
        },
        layout: wit::Layout {
            direction: match s.layout.direction {
                Direction::Column => wit::Direction::Column,
                Direction::Row => wit::Direction::Row,
            },
            gap: s.layout.gap,
            padding: s.layout.padding,
            align: match s.layout.align {
                Align::Start => wit::Align::Start,
                Align::Center => wit::Align::Center,
                Align::End => wit::Align::End,
                Align::Stretch => wit::Align::Stretch,
            },
            justify: match s.layout.justify {
                Justify::Start => wit::Justify::Start,
                Justify::Center => wit::Justify::Center,
                Justify::End => wit::Justify::End,
                Justify::SpaceBetween => wit::Justify::SpaceBetween,
            },
            grow: s.layout.grow,
            wrap: s.layout.wrap,
        },
        muted: s.muted,
    }
}

fn semantics(s: uniwidged_view::Semantics) -> wit::Semantics {
    use uniwidged_view::Interaction;
    wit::Semantics {
        label: s.label,
        interaction: match s.interaction {
            Interaction::None => wit::Interaction::None,
            Interaction::Button => wit::Interaction::Button,
            Interaction::Toggle => wit::Interaction::Toggle,
            Interaction::ListItem => wit::Interaction::ListItem,
            Interaction::Link(url) => wit::Interaction::Link(url),
        },
        state: wit::State {
            disabled: s.state.disabled,
            selected: s.state.selected,
            checked: s.state.checked,
        },
        tooltip: s.tooltip,
    }
}

pub fn wake_to_wire(wake: Wake) -> wit::Wake {
    match wake {
        Wake::Never => wit::Wake::Never,
        Wake::At(instant) => {
            // Before 1970 cannot be expressed and is already past anyway, so it
            // becomes "immediately" rather than silently becoming "never" —
            // which would stop the widget for good.
            let seconds = instant
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            wit::Wake::At(seconds)
        }
        Wake::After(delay) => wit::Wake::AfterMs(delay.as_millis().min(u32::MAX as u128) as u32),
        Wake::Animating => wit::Wake::Animating,
    }
}

pub fn event_from_wire(event: wit::Event) -> Event {
    match event {
        wit::Event::Clicked(id) => Event::Clicked(NodeId(id)),
        wit::Event::Toggled((id, on)) => Event::Toggled(NodeId(id), on),
        wit::Event::Delivered(token) => Event::Delivered { token },
        wit::Event::Configured => Event::Configured,
        wit::Event::EnvironmentChanged => Event::EnvironmentChanged,
    }
}
