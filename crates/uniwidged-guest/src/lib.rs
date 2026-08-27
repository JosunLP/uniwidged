//! Write a widget.
//!
//! This is the whole surface a widget author sees. It compiles to
//! `wasm32-wasip2` and shares its types with the host, so the tree you build
//! here is the tree the compositor lays out — there is no schema in between
//! and nothing to keep in sync.
//!
//! ```ignore
//! use uniwidged_guest::prelude::*;
//!
//! struct Clock;
//!
//! impl Widget for Clock {
//!     fn view(&mut self, cx: &Cx) -> View {
//!         let now = cx.local_time();
//!         panel(NodeId::named("clock"))
//!             .child(text(cx.format_time(now)).role(Role::Display))
//!             .child(text(cx.format_date(now)).role(Role::Caption).muted())
//!             .view()
//!     }
//!
//!     // The top of the next minute — not "in 60 seconds", so the clock stays
//!     // right across a suspend, a timezone change and an NTP step.
//!     fn wake(&self) -> Wake { Wake::At(next_minute()) }
//! }
//!
//! uniwidged_guest::export!(Clock);
//! ```
//!
//! ## What you do not do
//!
//! You do not draw, choose a colour, pick a font, measure a pixel or handle a
//! coordinate. You describe; the host decides. `ARCHITECTURE.md` §6 has the
//! argument, and the short version is that this is what makes a screenful of
//! widgets from different authors look like one product.
//!
//! You also do not poll. If you are waiting for something — an HTTP response,
//! a file to change — you ask for it and return [`Wake::Never`]; the host
//! wakes you when it arrives. A widget that spins is the one way to make a
//! desktop full of widgets cost something, and the host will throttle you for
//! it.
//!
//! ## Status
//!
//! Skeleton — the trait is the contract; the `export!` macro and the WIT
//! bindings land in milestone M3.

#![allow(dead_code)]

pub mod prelude;

use uniwidged_core::{NodeId, Wake};
use uniwidged_view::View;

/// Something that happened to your widget.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A node you marked interactive was activated. Reported by the id you
    /// gave it, never by coordinate — the host did the layout, so the host
    /// does the hit-testing.
    Clicked(NodeId),
    Toggled(NodeId, bool),
    /// A capability you asked for has an answer. Correlated by the token the
    /// request returned.
    Delivered {
        token: u32,
    },
    /// The user changed your configuration.
    Configured,
    /// The desktop's appearance, contrast or locale changed. Your view will be
    /// rebuilt; you rarely need to do anything.
    EnvironmentChanged,
}

/// The host, as a widget sees it.
///
/// Every method here is backed by a capability. Calling one you were not
/// granted returns an error rather than trapping, so a widget can degrade —
/// a weather widget without `net` should show its last reading and say it is
/// stale, not disappear.
///
/// Skeleton: the methods land with the WIT bindings in M3.
#[derive(Debug)]
pub struct Cx {
    _private: (),
}

/// The one trait a widget implements.
pub trait Widget {
    /// Describe what should be on screen.
    ///
    /// Called whenever the host thinks something may have changed. Return the
    /// same tree and nothing is drawn — the comparison is
    /// [`View::differs_from`], so an unchanged widget costs one tree
    /// comparison and no GPU work at all. Build the tree fresh every time;
    /// caching it yourself is not the optimisation it looks like.
    fn view(&mut self, cx: &Cx) -> View;

    /// When you would like to be called again.
    ///
    /// Defaults to [`Wake::Never`], which is right more often than it looks.
    fn wake(&self) -> Wake {
        Wake::Never
    }

    /// React to something. Return `true` if your view would now differ.
    ///
    /// Returning `false` when it would have changed means a stale widget;
    /// returning `true` when it would not costs one tree comparison. When in
    /// doubt, return `true`.
    fn on_event(&mut self, _event: &Event, _cx: &Cx) -> bool {
        false
    }
}
