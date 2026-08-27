//! View tree in, frame out.
//!
//! This is where a widget's description of what it wants becomes something
//! that can be drawn, clicked and read aloud. It is the largest pure-logic
//! crate in the project and it has no GPU, no window and no OS in it, so all
//! of it is testable headlessly.
//!
//! One pass over the tree produces three outputs, and they have to agree with
//! each other:
//!
//! * a `vello::Scene` — what gets drawn,
//! * absolute hit rectangles per [`uniwidged_core::NodeId`] — what a click
//!   lands on,
//! * AccessKit nodes with the same ids and the same rectangles — what a screen
//!   reader is told.
//!
//! Producing them together rather than in three passes is not an optimisation.
//! It is the only way they cannot drift: a screen reader that announces a
//! button where a click does nothing is worse than no screen reader support.
//!
//! ## The theme resolves, the widget names
//!
//! A [`uniwidged_view::Role`] becomes a font size, weight and opacity here. A
//! [`uniwidged_view::Accent`] becomes a colour here, in Oklch, against the
//! current appearance and contrast setting. `gap(2)` becomes pixels here. The
//! widget named things; this crate decides what they are. See
//! `ARCHITECTURE.md` §6.
//!
//! ## Status
//!
//! Skeleton. The type signatures below are the contract the rest of the
//! workspace is written against; the bodies land in milestone M2 of
//! `ROADMAP.md`, on top of `ferrum-layout` (taffy) and `ferrum-text` (Parley).

#![allow(dead_code)]

pub mod theme;

use std::collections::HashMap;
use uniwidged_core::{NodeId, Rect, Size};
use uniwidged_view::View;

/// Everything one pass over a view tree produces.
///
/// Deliberately one struct rather than three return values: see the module
/// note on why these may never be computed separately.
#[derive(Debug, Default)]
pub struct Composed {
    /// Where each named node ended up, in logical pixels relative to the
    /// surface's top-left. Anonymous nodes are absent — nothing can address
    /// them, so nothing needs their rectangle.
    pub hit_rects: HashMap<NodeId, Rect>,
    /// The size the tree wants, before the surface's own constraints.
    pub intrinsic: Size,
    // scene: vello::Scene           — M2
    // a11y: accesskit::TreeUpdate   — M2
}

/// Lays out, shapes and paints one view tree.
///
/// Owns the font context and the layout arena, so it is created once per host
/// and reused for every widget and every frame. Parley's font collection in
/// particular is expensive to build and must not be per-widget: eight widgets
/// each enumerating the system's fonts at start-up would be visible.
#[derive(Debug, Default)]
pub struct Compositor {
    theme: theme::Theme,
}

impl Compositor {
    pub fn new(theme: theme::Theme) -> Self {
        Self { theme }
    }

    pub fn theme(&self) -> &theme::Theme {
        &self.theme
    }

    /// Composes `view` into a frame at most `available` logical pixels.
    pub fn compose(&mut self, _view: &View, available: Size) -> Composed {
        // M2: taffy layout → Parley shaping → Vello scene → hit rects → a11y.
        Composed {
            intrinsic: available,
            ..Default::default()
        }
    }
}
