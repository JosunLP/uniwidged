//! Everything a widget needs, in one `use`.
//!
//! A widget file should start with `use uniwidged_guest::prelude::*;` and then
//! be about the widget.

pub use crate::{Cx, Event, Widget};
pub use uniwidged_core::{NodeId, Wake};
pub use uniwidged_view::build::*;
pub use uniwidged_view::{Accent, Align, Icon, Justify, Role, View};
