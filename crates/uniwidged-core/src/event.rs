//! Input, without a platform in it.
//!
//! `uniwidged-shell` translates an `XEvent`, a `WPARAM`, an `NSEvent` or a
//! `wl_pointer` callback into these, and nothing above it ever sees the
//! original. That is what lets the whole stack above the shell be tested with
//! no display attached.
//!
//! These are *host* events. They do not cross the sandbox boundary: a widget
//! is told `clicked(NodeId)`, not "button 1 went down at (37.5, 12.0)". The
//! host hit-tests, because the host is the one that did the layout and is
//! therefore the only participant that knows where anything is.

use crate::geom::Point;

/// Which mouse button, in the names a user would use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
    Back,
    Forward,
}

/// What the pointer just did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase {
    Entered,
    Moved,
    Down(PointerButton),
    Up(PointerButton),
    /// The pointer left the surface. Any hover state has to be dropped: on a
    /// desktop widget this arrives far more often than on an application
    /// window, because the pointer crosses the thing on its way elsewhere.
    Left,
    Scrolled(ScrollDelta),
}

/// How far a scroll went, in whichever unit the device reports.
///
/// Kept as two variants rather than normalised to pixels because the
/// conversion needs a line height, and only the layout knows that. A trackpad
/// reports pixels; a wheel reports detents.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollDelta {
    Lines { x: f32, y: f32 },
    Pixels { x: f64, y: f64 },
}

impl Eq for ScrollDelta {}

/// A pointer event in **logical** pixels, relative to the surface's top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEvent {
    pub position: Point,
    pub phase: PointerPhase,
    pub modifiers: Modifiers,
}

/// The modifier keys held down, named by role rather than by keycap.
///
/// `command` is the platform's primary accelerator: Ctrl on Windows and Linux,
/// Cmd on macOS. Resolving it in the shell rather than in every consumer is
/// what keeps a shortcut definition portable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub command: bool,
    pub alt: bool,
    /// The physical Ctrl key, for the rare binding that means Ctrl on macOS
    /// too. Equal to `command` everywhere else.
    pub control: bool,
}

/// A key that is not a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Space,
}

/// What a key press produced: text, or a named key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    /// The characters the key produced once the input method was done with it
    /// — which is why this is a `String` and not a `char`.
    Text(String),
    Named(NamedKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub pressed: bool,
    pub repeat: bool,
    pub modifiers: Modifiers,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accelerator_is_named_by_role_not_by_keycap() {
        // A shortcut written once has to mean Ctrl on Windows and Cmd on
        // macOS, so consumers ask for `command` and never for either keycap.
        let m = Modifiers {
            command: true,
            ..Default::default()
        };
        assert!(m.command);
        assert!(!m.shift && !m.alt);
    }
}
