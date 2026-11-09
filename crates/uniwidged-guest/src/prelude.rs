//! Everything a widget needs, in one `use`.
//!
//! A widget file should start with `use uniwidged_guest::prelude::*;` and then
//! be about the widget.

pub use crate::{
    Battery, CommandOutput, Cx, Denied, Event, HttpResponse, PowerState, SettingValue, TaskResult,
    Throughput, Widget, now_seconds,
};
pub use uniwidged_core::{NodeId, Wake};
pub use uniwidged_view::View;
pub use uniwidged_view::build::*;
