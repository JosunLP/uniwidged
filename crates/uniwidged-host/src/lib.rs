//! The daemon that owns everything.
//!
//! One process, one GPU device, one event loop, N widgets. It composes the
//! shell, the renderer, the compositor and the WASM runtime, and it owns the
//! scheduler — which is the piece that decides whether a desktop full of
//! widgets costs nothing or costs a battery.
//!
//! ## Status
//!
//! [`scheduler`] is complete and tested. Lifecycle, configuration and IPC land
//! in milestone M7.

pub mod config;
pub mod host;
pub mod scheduler;
pub mod supervisor;

pub use config::{Config, Placement, WidgetConfig};
pub use host::{Host, HostError, TickReport};
pub use scheduler::{Scheduler, Tick};
pub use supervisor::{State, Supervised};
