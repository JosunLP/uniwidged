//! Moving and resizing a widget with the pointer.
//!
//! A desktop widget is furniture: it sits where the user put it, and putting it
//! there is a drag. That makes the pointer ambiguous — the same press could be
//! the start of a drag or a click on a button — so this is the piece that
//! decides, and it decides from things the host already knows.
//!
//! ## The rule
//!
//! 1. **Locked** — the user has said the widget stays put. Every press is the
//!    widget's, and nothing moves.
//! 2. **On the frame** — within [`GRIP`] logical pixels of an edge, a press
//!    resizes. The band wins even over a button, because a control sitting on
//!    the very edge would otherwise make the widget unresizable from that side,
//!    and there is nowhere else to grab.
//! 3. **On something interactive** — a button, a toggle, a list item, a link.
//!    The press belongs to the widget. `uniwidged-compose` records which nodes
//!    those are in the same pass that records the hit rectangles.
//! 4. **Anywhere else** — a drag.
//!
//! A press is not yet a gesture. Nothing happens until the pointer has moved
//! [`THRESHOLD`] logical pixels; before that the press is still a click, which
//! is why a click is delivered on *release* rather than on press. Pressing a
//! button and dragging away cancels it, as it does everywhere else.
//!
//! ## Root coordinates, not window ones
//!
//! Every position here is in the **desktop's** coordinate space, not the
//! window's. A drag computed from window-relative coordinates feeds the
//! window's own movement back into its input: the pointer moves ten pixels, the
//! window moves ten pixels, the pointer is where it started relative to the
//! window, and the widget shivers instead of moving. X11 gives both; the shell
//! passes both, and only `local` — used once, to find the edge — is
//! window-relative.

use uniwidged_core::WidgetId;
use uniwidged_core::geom::PhysicalRect;

/// How far the pointer must travel before a press becomes a drag, in logical
/// pixels.
///
/// Below this, a hand that is not quite steady turns every click into a
/// one-pixel move — and a widget that shifts when you click it feels broken in
/// a way nobody can quite name.
pub const THRESHOLD: f64 = 4.0;

/// The width of the resize band inside each edge, in logical pixels.
pub const GRIP: f64 = 6.0;

/// The smallest a widget may be dragged down to, in logical pixels.
///
/// Not a matter of taste: below this the panel's own padding exceeds its box,
/// there is nothing left to grab, and the widget cannot be made bigger again
/// with the pointer.
pub const MIN_SIZE: (u32, u32) = (96, 64);

/// How much of a widget must stay on screen, in logical pixels.
///
/// A widget dragged entirely off the edge is a widget the user cannot get
/// back, because there is nothing left to grab. This is the amount that always
/// remains reachable.
pub const KEEP_ON_SCREEN: f64 = 32.0;

/// Which part of a widget's frame the pointer is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Edge {
    /// The edge a point is on, given the widget's size. All in physical pixels.
    ///
    /// Corners take priority over sides, which is what makes a corner a corner:
    /// the overlap belongs to the diagonal, or the two-pixel-wide sliver where
    /// they meet would resize only one axis.
    pub fn at(local: (i32, i32), size: (u32, u32), grip: f64) -> Option<Edge> {
        let (x, y) = (local.0 as f64, local.1 as f64);
        let (width, height) = (size.0 as f64, size.1 as f64);
        if x < 0.0 || y < 0.0 || x > width || y > height {
            return None;
        }
        // A widget can be narrower than two grips. Splitting the difference
        // keeps both sides reachable instead of letting the left edge win the
        // whole width.
        let grip_x = grip.min(width / 2.0);
        let grip_y = grip.min(height / 2.0);
        let left = x <= grip_x;
        let right = x >= width - grip_x;
        let top = y <= grip_y;
        let bottom = y >= height - grip_y;
        Some(match (left, right, top, bottom) {
            (true, _, true, _) => Edge::TopLeft,
            (_, true, true, _) => Edge::TopRight,
            (true, _, _, true) => Edge::BottomLeft,
            (_, true, _, true) => Edge::BottomRight,
            (true, _, _, _) => Edge::Left,
            (_, true, _, _) => Edge::Right,
            (_, _, true, _) => Edge::Top,
            (_, _, _, true) => Edge::Bottom,
            _ => return None,
        })
    }

    fn moves_left(self) -> bool {
        matches!(self, Edge::Left | Edge::TopLeft | Edge::BottomLeft)
    }

    fn moves_right(self) -> bool {
        matches!(self, Edge::Right | Edge::TopRight | Edge::BottomRight)
    }

    fn moves_top(self) -> bool {
        matches!(self, Edge::Top | Edge::TopLeft | Edge::TopRight)
    }

    fn moves_bottom(self) -> bool {
        matches!(self, Edge::Bottom | Edge::BottomLeft | Edge::BottomRight)
    }
}

/// What a press turned out to be for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    /// The widget's. Delivered as a click on release, unless the pointer
    /// wanders off first.
    Click,
    Move,
    Resize(Edge),
}

/// What the host should do about a press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// The widget's. Nothing is dragged; a click follows on release.
    ToWidget,
    /// A move may be starting. Grab the pointer.
    Move,
    /// A resize may be starting, from this edge.
    Resize(Edge),
}

/// What a release turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Release {
    /// The pointer never travelled far enough to be a drag, so it was a click.
    /// The host delivers it at the point of the *press*, not the release —
    /// they differ by a pixel or two, and the node under the press is the one
    /// the user aimed at.
    Click { widget: WidgetId, at: (i32, i32) },
    /// A move or resize finished. This is the geometry to keep, and to write
    /// back to the configuration.
    Placed {
        widget: WidgetId,
        rect: PhysicalRect,
    },
    /// A press that became a drag on something interactive, and is therefore
    /// neither a click nor a placement.
    Nothing,
}

/// One press in progress.
#[derive(Debug, Clone)]
struct Held {
    widget: WidgetId,
    /// Desktop coordinates of the press.
    from: (i32, i32),
    /// Window coordinates of the press, kept so a click can be delivered where
    /// the user aimed.
    local: (i32, i32),
    /// The widget's geometry when the press happened. Every intermediate
    /// rectangle is computed from *this* rather than from the last one, so a
    /// drag cannot accumulate rounding.
    origin: PhysicalRect,
    intent: Intent,
    /// Set once the pointer has passed [`THRESHOLD`]. Until then the press is
    /// still a click.
    started: bool,
}

/// The pointer, as far as widget placement is concerned.
///
/// One per desktop rather than one per widget: only one thing can be dragged
/// at a time, and a second press arriving mid-drag is a bug in the caller
/// rather than a second gesture.
#[derive(Debug, Clone)]
pub struct Interaction {
    /// Device pixels per logical pixel. Everything below is physical, because
    /// window geometry is; the constants above are logical and are scaled here.
    scale: f64,
    /// The desktop, if the caller knows it. Used only to keep a widget
    /// reachable — see [`KEEP_ON_SCREEN`].
    bounds: Option<PhysicalRect>,
    held: Option<Held>,
}

impl Interaction {
    pub fn new(scale: f64) -> Self {
        Self {
            scale: if scale > 0.0 { scale } else { 1.0 },
            bounds: None,
            held: None,
        }
    }

    pub fn set_scale(&mut self, scale: f64) {
        if scale > 0.0 {
            self.scale = scale;
        }
    }

    /// The desktop's usable area, so a widget cannot be dragged out of reach.
    pub fn set_bounds(&mut self, bounds: Option<PhysicalRect>) {
        self.bounds = bounds;
    }

    /// The widget a gesture is currently on, if any.
    ///
    /// The caller routes motion by this rather than by the window the event
    /// arrived on: during a pointer grab the events all come from one window,
    /// and after a fast drag the pointer may be over a different one entirely.
    pub fn widget(&self) -> Option<WidgetId> {
        self.held.as_ref().map(|held| held.widget)
    }

    /// Whether a drag or resize is actually under way, as opposed to a press
    /// that has not moved.
    pub fn is_dragging(&self) -> bool {
        self.held
            .as_ref()
            .is_some_and(|held| held.started && held.intent != Intent::Click)
    }

    /// The pointer went down on a widget.
    ///
    /// `local` is in the window's own physical pixels and is used once, to find
    /// the edge; `root` is the desktop position everything else is measured
    /// from. `on_interactive` is the host's answer to "does the node under this
    /// point want clicks?".
    pub fn press(
        &mut self,
        widget: WidgetId,
        root: (i32, i32),
        local: (i32, i32),
        geometry: PhysicalRect,
        locked: bool,
        on_interactive: bool,
    ) -> Press {
        let intent = if locked {
            Intent::Click
        } else if let Some(edge) = Edge::at(local, (geometry.width, geometry.height), self.grip()) {
            Intent::Resize(edge)
        } else if on_interactive {
            Intent::Click
        } else {
            Intent::Move
        };
        self.held = Some(Held {
            widget,
            from: root,
            local,
            origin: geometry,
            intent,
            started: false,
        });
        match intent {
            Intent::Click => Press::ToWidget,
            Intent::Move => Press::Move,
            Intent::Resize(edge) => Press::Resize(edge),
        }
    }

    /// The pointer moved. `Some` is the geometry the window should take now.
    pub fn motion(&mut self, root: (i32, i32)) -> Option<PhysicalRect> {
        let held = self.held.as_mut()?;
        let delta = (root.0 - held.from.0, root.1 - held.from.1);
        if !held.started {
            let travelled = ((delta.0 * delta.0 + delta.1 * delta.1) as f64).sqrt();
            if travelled < THRESHOLD * self.scale {
                return None;
            }
            held.started = true;
        }
        let rect = match held.intent {
            // A press that wandered off a button is no longer a click, and
            // there is nothing else for it to be.
            Intent::Click => return None,
            Intent::Move => PhysicalRect::new(
                held.origin.x + delta.0,
                held.origin.y + delta.1,
                held.origin.width,
                held.origin.height,
            ),
            Intent::Resize(edge) => resize(held.origin, edge, delta, self.min()),
        };
        Some(self.contain(rect))
    }

    /// The pointer came up.
    pub fn release(&mut self, root: (i32, i32)) -> Release {
        let Some(held) = self.held.take() else {
            return Release::Nothing;
        };
        if !held.started {
            return Release::Click {
                widget: held.widget,
                at: held.local,
            };
        }
        let delta = (root.0 - held.from.0, root.1 - held.from.1);
        match held.intent {
            Intent::Click => Release::Nothing,
            Intent::Move => Release::Placed {
                widget: held.widget,
                rect: self.contain(PhysicalRect::new(
                    held.origin.x + delta.0,
                    held.origin.y + delta.1,
                    held.origin.width,
                    held.origin.height,
                )),
            },
            Intent::Resize(edge) => Release::Placed {
                widget: held.widget,
                rect: self.contain(resize(held.origin, edge, delta, self.min())),
            },
        }
    }

    /// Abandons a gesture without applying it.
    ///
    /// For a window that went away mid-drag, or a session that ended. The
    /// widget keeps the geometry it had when the press happened, which the
    /// caller still holds — nothing here has been written back.
    pub fn cancel(&mut self) -> Option<PhysicalRect> {
        self.held.take().map(|held| held.origin)
    }

    fn grip(&self) -> f64 {
        GRIP * self.scale
    }

    fn min(&self) -> (u32, u32) {
        (
            (MIN_SIZE.0 as f64 * self.scale).round() as u32,
            (MIN_SIZE.1 as f64 * self.scale).round() as u32,
        )
    }

    /// Keeps at least a corner of the widget inside the desktop.
    fn contain(&self, rect: PhysicalRect) -> PhysicalRect {
        let Some(bounds) = self.bounds else {
            return rect;
        };
        let keep = (KEEP_ON_SCREEN * self.scale) as i32;
        // The widget may be wider than the margin, so the limits are expressed
        // in terms of how far its *near* edge may go.
        let min_x = bounds.x - rect.width as i32 + keep;
        let max_x = bounds.x + bounds.width as i32 - keep;
        // Never above the top: a titlebarless widget dragged upward leaves
        // nothing behind, and the desktop's top edge is where panels live.
        let min_y = bounds.y;
        let max_y = bounds.y + bounds.height as i32 - keep;
        PhysicalRect::new(
            rect.x.clamp(min_x.min(max_x), max_x.max(min_x)),
            rect.y.clamp(min_y.min(max_y), max_y.max(min_y)),
            rect.width,
            rect.height,
        )
    }
}

/// The rectangle a resize produces, clamped so it never inverts.
///
/// Dragging the left edge past the right one must stop at the minimum width
/// rather than turning the widget inside out — and stopping means the *edge*
/// stops, so the opposite side stays exactly where it was.
fn resize(origin: PhysicalRect, edge: Edge, delta: (i32, i32), min: (u32, u32)) -> PhysicalRect {
    let (mut x, mut y) = (origin.x, origin.y);
    let (mut width, mut height) = (origin.width as i32, origin.height as i32);
    let (min_w, min_h) = (min.0 as i32, min.1 as i32);

    if edge.moves_left() {
        // How far the edge may travel before the box hits its minimum.
        let travel = delta.0.min(width - min_w);
        x += travel;
        width -= travel;
    } else if edge.moves_right() {
        width = (width + delta.0).max(min_w);
    }
    if edge.moves_top() {
        let travel = delta.1.min(height - min_h);
        y += travel;
        height -= travel;
    } else if edge.moves_bottom() {
        height = (height + delta.1).max(min_h);
    }
    PhysicalRect::new(x, y, width.max(min_w) as u32, height.max(min_h) as u32)
}

#[cfg(test)]
mod tests;
