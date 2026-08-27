//! A reference widget, so that "what does a widget look like?" has an answer
//! that compiles.
//!
//! This is a system-load panel: a title, three labelled bars, a source line and
//! a refresh button. It exercises most of the vocabulary — rows, spacers,
//! progress, chips, accents, muting, tooltips and an interactive node — in
//! about forty lines of description and no drawing at all.
//!
//! What is *not* here is the point. There is no colour, no font, no pixel and
//! no coordinate: the widget names [`Role::Display`] and the theme decides
//! what that is. See `ARCHITECTURE.md` §6.
//!
//! ## The full widget, from milestone M3
//!
//! Once the WASM bindings land, the trait implementation around [`view`] is:
//!
//! ```ignore
//! use uniwidged_guest::prelude::*;
//!
//! struct SystemLoad { reading: Reading }
//!
//! impl Widget for SystemLoad {
//!     fn view(&mut self, _cx: &Cx) -> View {
//!         hello_widget::view(&self.reading)
//!     }
//!
//!     // Every two seconds while visible. Not a poll: the host sleeps, and
//!     // nothing at all runs in between.
//!     fn wake(&self) -> Wake { Wake::After(Duration::from_secs(2)) }
//!
//!     fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {
//!         match event {
//!             Event::Clicked(id) if *id == NodeId::named("refresh") => {
//!                 self.reading = read_metrics(cx);
//!                 true                    // the view will differ
//!             }
//!             _ => false,                 // nothing changed; no frame
//!         }
//!     }
//! }
//!
//! uniwidged_guest::export!(SystemLoad);
//! ```

use uniwidged_core::NodeId;
use uniwidged_view::build::*;

/// What the widget has to show. Fractions are `0.0..=1.0`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Reading {
    pub cpu: f32,
    pub memory: f32,
    pub disk: f32,
    /// Whether the last read succeeded. A widget that cannot refresh should
    /// show its last value and say it is stale — not disappear, and not lie.
    pub stale: bool,
}

/// The whole widget, as a description.
pub fn view(reading: &Reading) -> View {
    panel(NodeId::named("system-load"))
        .gap(2)
        .child(
            row()
                .child(text("System").role(Role::Title))
                .child(spacer())
                .child(stale_marker(reading.stale))
                .child(
                    icon(Icon::Refresh)
                        .on_click(NodeId::named("refresh"))
                        .label("Refresh now")
                        .tooltip("Read the counters again"),
                ),
        )
        .child(divider())
        .child(
            column()
                .gap(1)
                .child(meter("CPU", reading.cpu))
                .child(meter("Memory", reading.memory))
                .child(meter("Disk", reading.disk)),
        )
        .view()
}

/// One labelled bar: name on the left, percentage on the right, bar beneath.
///
/// The accent is the only judgement the widget makes about colour, and it
/// makes it in terms of meaning — "this is critical" — rather than of hue.
fn meter(name: &str, value: f32) -> Node {
    column()
        .gap(0)
        .child(
            row()
                .child(text(name).role(Role::Caption))
                .child(spacer())
                .child(
                    text(format!("{:.0}%", value * 100.0))
                        .role(Role::Caption)
                        .accent(severity(value)),
                ),
        )
        .child(progress(value).accent(severity(value)))
}

/// Where a fraction stops being fine.
///
/// Thresholds rather than a gradient, because the point of the colour is to be
/// noticed from across a room, and a continuous ramp is never noticed.
fn severity(value: f32) -> Accent {
    if value >= 0.9 {
        Accent::Critical
    } else if value >= 0.75 {
        Accent::Warning
    } else {
        Accent::Neutral
    }
}

/// Says the reading is old, and nothing at all when it is not.
///
/// A [`Content::Spacer`](uniwidged_view::Content::Spacer) with no growth is an
/// empty node — the honest way to say "nothing here" without the caller having
/// to branch.
fn stale_marker(stale: bool) -> Node {
    if stale {
        chip("stale")
            .accent(Accent::Warning)
            .tooltip("The last read failed; this is the previous value")
    } else {
        spacer().grow(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(node: &Node, id: NodeId) -> Option<&Node> {
        if node.id == id {
            return Some(node);
        }
        node.children.iter().find_map(|c| find(c, id))
    }

    #[test]
    fn a_calm_machine_shows_no_warning_colour() {
        let calm = Reading {
            cpu: 0.12,
            memory: 0.44,
            disk: 0.61,
            stale: false,
        };
        assert_eq!(severity(calm.cpu), Accent::Neutral);
        assert_eq!(severity(calm.memory), Accent::Neutral);
        assert_eq!(severity(calm.disk), Accent::Neutral);
    }

    #[test]
    fn severity_escalates_at_its_thresholds_and_not_before() {
        assert_eq!(severity(0.749), Accent::Neutral);
        assert_eq!(severity(0.75), Accent::Warning);
        assert_eq!(severity(0.899), Accent::Warning);
        assert_eq!(severity(0.9), Accent::Critical);
        assert_eq!(severity(1.0), Accent::Critical);
    }

    #[test]
    fn the_refresh_button_is_addressable() {
        // The click is reported by id, so the node has to carry one — and a
        // screen reader needs the label, because a glyph reads as nothing.
        let v = view(&Reading::default());
        let refresh = find(&v.root, NodeId::named("refresh")).expect("refresh node");
        assert_eq!(refresh.semantics.interaction, Interaction::Button);
        assert_eq!(refresh.semantics.label.as_deref(), Some("Refresh now"));
        assert_eq!(refresh.content, Content::Icon(Icon::Refresh));
    }

    #[test]
    fn a_stale_reading_says_so_rather_than_disappearing() {
        let fresh = view(&Reading::default());
        let stale = view(&Reading {
            stale: true,
            ..Default::default()
        });
        // The layout keeps its shape either way — a marker appearing must not
        // make the other rows jump.
        assert_eq!(fresh.root.children.len(), stale.root.children.len());
        assert!(fresh.differs_from(&stale));
    }

    #[test]
    fn an_unchanged_reading_produces_no_frame() {
        // The damage model, from the widget author's side: describing the same
        // state twice costs one tree comparison and no GPU work.
        let reading = Reading {
            cpu: 0.3,
            memory: 0.5,
            disk: 0.7,
            stale: false,
        };
        assert!(!view(&reading).differs_from(&view(&reading)));
    }
}
