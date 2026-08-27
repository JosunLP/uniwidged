//! The vocabulary every other uniwidged crate speaks.
//!
//! This crate sits at the bottom of the stack and has no operating system, no
//! GPU and no windowing dependency — only geometry ([`kurbo`]) and colour
//! ([`peniko`]). It compiles fast, it compiles to `wasm32-wasip2`, and it can
//! be used from tests and tooling with nothing installed.
//!
//! Compiling to WASM is not incidental. The guest SDK a widget author writes
//! against shares these types with the host, so a `Rect` a widget reasons
//! about and a `Rect` the compositor lays out are the *same type* rather than
//! two definitions kept in sync by hand.
//!
//! * [`geom`] — geometry and colour, re-exported from the Linebender crates so
//!   the whole framework shares one `Point`/`Rect`/`Color` and never converts.
//! * [`id`] — stable identities for widgets and the nodes inside them. Also
//!   the basis for AccessKit node ids, which is why they have to survive a
//!   rebuild.
//! * [`event`] — a platform-independent input model. `uniwidged-shell`
//!   translates native events into these, so nothing above it ever sees an
//!   `XEvent`, a `WPARAM` or an `NSEvent`.
//! * [`wake`] — when the host is allowed to do work again. The single most
//!   important type here: see `ARCHITECTURE.md` §5.

pub mod event;
pub mod geom;
pub mod id;
pub mod wake;

pub use event::{
    Key, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent, PointerPhase, ScrollDelta,
};
pub use geom::{Color, Insets, Point, Rect, Size, Vec2};
pub use id::{NodeId, WidgetId};
pub use wake::{Deadline, Wake};
