//! View tree in, frame out.
//!
//! This is where a widget's description of what it wants becomes something
//! that can be drawn, clicked and read aloud. It is the largest pure-logic
//! crate in the project: no GPU, no window, no operating system, so all of it
//! is testable headlessly and the golden-image suite is the only part that
//! needs a device.
//!
//! One pass over the tree produces three outputs, and they have to agree:
//!
//! * a [`Scene`] — what gets drawn,
//! * absolute hit rectangles per [`NodeId`] — what a click lands on,
//! * AccessKit nodes with the same ids and the same rectangles — what a screen
//!   reader is told.
//!
//! Producing them together rather than in three passes is not an optimisation;
//! it is the only way they cannot drift. A screen reader that announces a
//! button where a click does nothing is worse than no screen reader support at
//! all, and three passes over a changing tree eventually produce exactly that.
//!
//! ## The theme resolves, the widget names
//!
//! A [`Role`](uniwidged_view::Role) becomes a size, weight and opacity here. An
//! [`Accent`](uniwidged_view::Accent) becomes a colour here, in Oklch, against
//! the current appearance and contrast setting. `gap(2)` becomes pixels here.
//! The widget named things; this crate decides what they are. See
//! `ARCHITECTURE.md` §6.

pub mod a11y;
pub mod icons;
pub mod layout;
pub mod paint;
pub mod text;
pub mod theme;

use std::collections::HashMap;

use uniwidged_core::{NodeId, Point, Rect, Size, WidgetId};
use uniwidged_render::Scene;
use uniwidged_view::View;

pub use layout::Placed;
pub use theme::{Appearance, Contrast, Theme};

/// Everything one pass over a view tree produces.
///
/// Deliberately one struct rather than three return values: see the module
/// note on why these may never be computed separately.
#[derive(Default)]
pub struct Composed {
    /// What to hand [`uniwidged_render`]. Already scaled to the surface's
    /// physical pixels.
    pub scene: Scene,
    /// Where each named node ended up, in **logical** pixels relative to the
    /// surface's top-left. Anonymous nodes are absent: nothing can address
    /// them, so nothing needs their rectangle.
    ///
    /// Logical rather than physical because that is the space input arrives in
    /// once the shell has divided out the scale factor, and converting twice is
    /// how a click ends up a few pixels off on a fractional-scale display.
    pub hit_rects: HashMap<NodeId, Rect>,
    /// Named nodes in paint order, so the topmost can be found without
    /// re-walking the tree. The last thing drawn is the thing on top.
    pub order: Vec<NodeId>,
    /// The flat node list for AccessKit, root first. See [`a11y`].
    pub a11y: Vec<(accesskit::NodeId, accesskit::Node)>,
    /// The size the tree wanted, before the surface's own constraints.
    pub intrinsic: Size,
}

impl Composed {
    /// What a click at `point` (logical pixels) landed on.
    ///
    /// Uses the recorded rectangles rather than re-walking the tree, so the
    /// answer is by construction the same one the screen reader was given.
    pub fn hit(&self, point: Point) -> Option<NodeId> {
        // Reverse paint order: the last thing drawn is the topmost.
        self.order
            .iter()
            .rev()
            .find(|id| {
                self.hit_rects
                    .get(*id)
                    .is_some_and(|rect| rect.contains(point))
            })
            .copied()
    }
}

/// Lays out, shapes and paints view trees.
///
/// Owns the font context and the theme, so it is created once per host and
/// reused for every widget and every frame. Parley's font collection in
/// particular is expensive to build and must not be per-widget: eight widgets
/// each enumerating the system's fonts at start-up would be visible.
pub struct Compositor {
    theme: Theme,
    text: text::TextEngine,
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new(Theme::default())
    }
}

impl Compositor {
    pub fn new(theme: Theme) -> Self {
        Self::with_text_engine(theme, text::TextEngine::new())
    }

    /// A compositor over a text engine the caller prepared.
    ///
    /// The golden-image suite uses it to supply a bundled font, so that a
    /// snapshot is a picture of this code rather than of whichever fonts the
    /// machine happened to have installed.
    pub fn with_text_engine(theme: Theme, text: text::TextEngine) -> Self {
        Self { theme, text }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Replaces the theme, for an appearance or contrast change.
    ///
    /// The shaping cache survives: a colour change does not alter a glyph
    /// position, and re-shaping every string on a light/dark switch would make
    /// the switch visibly slow with several widgets on screen.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    /// Composes one widget's view.
    ///
    /// `size` is the surface in **logical** pixels and `scale` is device pixels
    /// per logical pixel; the returned scene is already scaled, so the caller
    /// hands it straight to the renderer.
    pub fn compose(&mut self, request: Compose<'_>) -> Composed {
        let placed = layout::compute(
            request.view,
            &self.theme,
            &mut self.text,
            (request.size.width as f32, request.size.height as f32),
        );

        let mut scene = Scene::new();
        let painter = paint::Painter {
            theme: &self.theme,
            width: request.size.width,
            rtl: request.rtl,
            scale: request.scale,
        };
        painter.paint(&mut scene, &placed, &mut self.text);

        let mut hit_rects = HashMap::new();
        let mut order = Vec::new();
        placed.walk(&mut |node| {
            if !node.node.id.is_anonymous() {
                order.push(node.node.id);
                // Mirrored, because a click arrives in the coordinates the user
                // sees. Recording the unmirrored rectangle would make every
                // right-to-left widget clickable in the wrong place — silently,
                // and only for the users least likely to be testing it.
                hit_rects.insert(node.node.id, painter.mirror_rect(node.rect));
            }
        });

        let a11y = a11y::build(&placed, request.widget, request.label);

        Composed {
            scene,
            hit_rects,
            order,
            a11y,
            intrinsic: Size::new(placed.rect.width(), placed.rect.height()),
        }
    }

    /// Lays out without painting.
    ///
    /// For a host that needs a widget's natural size — to give a new widget a
    /// sensible default box — without paying for a scene it will not present.
    pub fn measure(&mut self, view: &View, available: Size) -> Size {
        let placed = layout::compute(
            view,
            &self.theme,
            &mut self.text,
            (available.width as f32, available.height as f32),
        );
        Size::new(placed.rect.width(), placed.rect.height())
    }
}

/// One request to compose.
///
/// A struct rather than six positional arguments, because four of them are
/// numbers and a caller that swaps two of them gets a widget that is subtly
/// wrong rather than a compile error.
pub struct Compose<'a> {
    pub view: &'a View,
    /// The widget instance, for accessibility ids that do not collide between
    /// two copies of the same widget.
    pub widget: WidgetId,
    /// What a screen reader calls this widget.
    pub label: &'a str,
    /// The surface, in logical pixels.
    pub size: Size,
    /// Device pixels per logical pixel.
    pub scale: f64,
    /// Mirror the layout for a right-to-left locale.
    pub rtl: bool,
}

impl<'a> Compose<'a> {
    pub fn new(view: &'a View, widget: WidgetId, label: &'a str, size: Size) -> Self {
        Self {
            view,
            widget,
            label,
            size,
            scale: 1.0,
            rtl: false,
        }
    }

    pub fn scale(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    pub fn rtl(mut self, rtl: bool) -> Self {
        self.rtl = rtl;
        self
    }
}

#[cfg(test)]
mod tests;
