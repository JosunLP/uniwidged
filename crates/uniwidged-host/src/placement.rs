//! Which monitor a widget is on, and where on it.
//!
//! A widget's position is stored **relative to a named output**, not as a
//! desktop coordinate. That is the whole of what makes a placement survive a
//! laptop being docked: monitor *indices* renumber, and desktop coordinates
//! move wholesale when a second screen is plugged in on the left, so a widget
//! remembered either way ends up somewhere its user did not put it — or off
//! the visible desktop entirely.
//!
//! ## What happens when the monitor is gone
//!
//! The widget is placed on the primary output at the same relative position,
//! clamped so it is reachable — **and the stored name is kept**. That is the
//! important half: undocking must not silently rewrite the configuration,
//! because then docking again would not put anything back. A widget only
//! forgets which output it belongs to when the user moves it themselves.

use uniwidged_core::geom::PhysicalRect;

use crate::config::Placement;

/// One monitor, as the platform reports it.
///
/// Defined by `uniwidged-shell`, because the platform is what knows: this crate
/// only decides what to do with the answer. Re-exported so a caller needs one
/// import rather than two.
pub use uniwidged_shell::Output;

/// Where a widget ended up, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Desktop coordinates, ready to hand a window system.
    pub rect: PhysicalRect,
    /// The output it was actually placed on, if any is known.
    pub output: Option<String>,
    /// Set when the widget asked for an output that is not here. The caller
    /// says so once, at start-up, rather than leaving a user to wonder why a
    /// widget is on the wrong screen.
    pub displaced: bool,
}

/// Turns a stored placement into a desktop rectangle.
pub fn resolve(stored: &Placement, outputs: &[Output]) -> Resolved {
    let rect = PhysicalRect::new(stored.x, stored.y, stored.width, stored.height);

    // Nothing to resolve against. A desktop that cannot enumerate its monitors
    // still has to run, and the stored coordinates are the best it has.
    if outputs.is_empty() {
        return Resolved {
            rect,
            output: None,
            displaced: false,
        };
    }

    let Some(name) = stored.output.as_deref() else {
        // No output was ever recorded, so the coordinates are already the
        // desktop's. This is what `uniwidged install` writes.
        return Resolved {
            rect,
            output: containing(rect, outputs).map(|o| o.name.clone()),
            displaced: false,
        };
    };

    match outputs.iter().find(|output| output.name == name) {
        Some(output) => Resolved {
            rect: PhysicalRect::new(
                output.rect.x + stored.x,
                output.rect.y + stored.y,
                stored.width,
                stored.height,
            ),
            output: Some(output.name.clone()),
            displaced: false,
        },
        None => {
            let fallback = primary(outputs);
            Resolved {
                rect: clamp_into(
                    PhysicalRect::new(
                        fallback.rect.x + stored.x,
                        fallback.rect.y + stored.y,
                        stored.width,
                        stored.height,
                    ),
                    fallback.rect,
                ),
                output: Some(fallback.name.clone()),
                displaced: true,
            }
        }
    }
}

/// The inverse: which output a widget is on now, and where on it.
///
/// What the desktop records after a drag. By the widget's **centre** rather
/// than its origin, because a widget straddling two monitors belongs to the one
/// showing most of it — and because an origin just off the left edge of a
/// screen would otherwise be attributed to the screen next door.
pub fn locate(rect: PhysicalRect, outputs: &[Output]) -> (Option<String>, PhysicalRect) {
    match containing(rect, outputs) {
        Some(output) => (
            Some(output.name.clone()),
            PhysicalRect::new(
                rect.x - output.rect.x,
                rect.y - output.rect.y,
                rect.width,
                rect.height,
            ),
        ),
        None => (None, rect),
    }
}

/// The output a rectangle's centre falls in.
fn containing(rect: PhysicalRect, outputs: &[Output]) -> Option<&Output> {
    let centre = (
        rect.x + rect.width as i32 / 2,
        rect.y + rect.height as i32 / 2,
    );
    outputs.iter().find(|output| {
        let area = output.rect;
        centre.0 >= area.x
            && centre.0 < area.x + area.width as i32
            && centre.1 >= area.y
            && centre.1 < area.y + area.height as i32
    })
}

/// The output marked primary, or the first one — a desktop always has one, and
/// this is only ever called with a non-empty list.
fn primary(outputs: &[Output]) -> &Output {
    outputs
        .iter()
        .find(|output| output.primary)
        .unwrap_or(&outputs[0])
}

/// Moves a rectangle inside a monitor, without resizing it.
///
/// A widget that was on a 4K screen and is now on a laptop's may be larger than
/// its new home. It keeps its size — shrinking somebody's widget because they
/// undocked is a change they did not ask for, and it would not grow back — and
/// is moved to the top-left corner so that what fits is visible.
fn clamp_into(rect: PhysicalRect, area: PhysicalRect) -> PhysicalRect {
    let max_x = area.x + area.width as i32 - rect.width as i32;
    let max_y = area.y + area.height as i32 - rect.height as i32;
    PhysicalRect::new(
        rect.x.clamp(area.x, max_x.max(area.x)),
        rect.y.clamp(area.y, max_y.max(area.y)),
        rect.width,
        rect.height,
    )
}

#[cfg(test)]
mod tests;
