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
pub mod interaction;
pub mod placement;
pub mod reload;
pub mod scheduler;
pub mod supervisor;

pub use config::{Config, Placement, WidgetConfig};
pub use host::{Activated, Host, HostError, TickReport};
pub use interaction::{Edge, Interaction, Press, Release};
pub use placement::{Output, Resolved};
pub use reload::{Applied, apply};
pub use scheduler::{Scheduler, Tick};
pub use supervisor::{Reconfigured, State, Supervised};
