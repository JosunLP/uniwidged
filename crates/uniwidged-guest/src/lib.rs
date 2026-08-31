//! Write a widget.
//!
//! This is the whole surface a widget author sees. It compiles to
//! `wasm32-wasip2` and shares its types with the host, so the tree you build
//! here is the tree the compositor lays out — there is no schema in between
//! and nothing to keep in sync.
//!
//! ```ignore
//! use uniwidged_guest::prelude::*;
//!
//! #[derive(Default)]
//! struct Clock { minute: u32 }
//!
//! impl Widget for Clock {
//!     fn view(&mut self, cx: &Cx) -> View {
//!         panel(NodeId::named("clock"))
//!             .child(text(cx.format_time(now()).unwrap_or_default()).role(Role::Display))
//!             .view()
//!     }
//!
//!     // The top of the next minute — not "in 60 seconds", so the clock stays
//!     // right across a suspend, a timezone change and an NTP step.
//!     fn wake(&self) -> Wake { Wake::At(next_minute()) }
//! }
//!
//! uniwidged_guest::export!(Clock);
//! ```
//!
//! ## What you do not do
//!
//! You do not draw, choose a colour, pick a font, measure a pixel or handle a
//! coordinate. You describe; the host decides. `ARCHITECTURE.md` §6 has the
//! argument, and the short version is that this is what makes a screenful of
//! widgets from different authors look like one product.
//!
//! You also do not poll. If you are waiting for something — an HTTP response,
//! a file to change — you ask for it and return [`Wake::Never`]; the host
//! wakes you when it arrives. A widget that spins is the one way to make a
//! desktop full of widgets cost something, and the host will throttle you for
//! it.
//!
//! ## Testing your widget
//!
//! Everything except [`Cx`] works natively, so the view a widget builds can be
//! unit-tested with no WASM toolchain at all — see `examples/hello_widget`.

#![allow(clippy::needless_doctest_main)]
#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub mod bindings;

pub mod prelude;

#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
pub use host::{CommandOutput, Cx, Denied, HttpResponse, SettingValue, TaskResult};

#[cfg(not(target_arch = "wasm32"))]
mod stub;
#[cfg(not(target_arch = "wasm32"))]
pub use stub::{CommandOutput, Cx, Denied, HttpResponse, SettingValue, TaskResult};

use uniwidged_core::{NodeId, Wake};
use uniwidged_view::View;

/// The ABI this SDK speaks. Must match the host's, or instantiation is refused.
pub const ABI_VERSION: u32 = 7;

/// Something that happened to your widget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A node you marked interactive was activated. Reported by the id you
    /// gave it, never by coordinate — the host did the layout, so the host
    /// does the hit-testing.
    Clicked(NodeId),
    Toggled(NodeId, bool),
    /// A capability you asked for has an answer, correlated by the token the
    /// request returned.
    Delivered {
        token: u32,
    },
    /// The user changed one of your settings. Read them again with
    /// [`Cx::setting`] and return `true` if your view would differ.
    Configured,
    /// The desktop's appearance, contrast or locale changed. Your view will be
    /// rebuilt; you rarely need to do anything.
    EnvironmentChanged,
}

/// The one trait a widget implements.
pub trait Widget: Default {
    /// Describe what should be on screen.
    ///
    /// Called whenever the host thinks something may have changed. Return the
    /// same tree and nothing is drawn — the comparison is
    /// [`View::differs_from`], so an unchanged widget costs one tree
    /// comparison and no GPU work at all. Build the tree fresh every time;
    /// caching it yourself is not the optimisation it looks like.
    fn view(&mut self, cx: &Cx) -> View;

    /// When you would like to be called again.
    ///
    /// Defaults to [`Wake::Never`], which is right more often than it looks.
    fn wake(&self) -> Wake {
        Wake::Never
    }

    /// Called once, before the first view.
    ///
    /// This is where a widget loads what it remembered — its cache, its saved
    /// state — which is why it is handed a [`Cx`]: a `start` that cannot reach
    /// the host can only initialise things the widget already knew, and every
    /// widget with anything to restore ended up doing it lazily in `view`
    /// behind a `loaded` flag instead.
    ///
    /// Returning an error stops the widget from starting at all, which is right
    /// for a configuration it cannot work without and wrong for anything it
    /// could show a message about. A missing cache is the second kind.
    fn start(&mut self, _cx: &Cx) -> Result<(), String> {
        Ok(())
    }

    /// React to something. Return `true` if your view would now differ.
    ///
    /// Returning `false` when it would have changed means a stale widget;
    /// returning `true` when it would not costs one tree comparison. When in
    /// doubt, return `true`.
    fn on_event(&mut self, _event: &Event, _cx: &Cx) -> bool {
        false
    }
}

/// Makes a type the widget this component exports.
///
/// Exactly one per component. On a non-wasm target it expands to nothing, so
/// that a widget crate still compiles — and still runs its own unit tests —
/// without a WASM toolchain.
#[cfg(target_arch = "wasm32")]
#[macro_export]
macro_rules! export {
    ($widget:ty) => {
        const _: () = {
            use ::std::cell::RefCell;

            thread_local! {
                // WebAssembly is single-threaded, so a thread-local is simply
                // "the instance". `RefCell` rather than a plain `static mut`
                // so that a re-entrant call — which the host does not make —
                // panics rather than aliasing.
                static WIDGET: RefCell<$widget> = RefCell::new(<$widget as ::core::default::Default>::default());
            }

            struct Component;

            impl $crate::bindings::exports::uniwidged::widget::guest::Guest for Component {
                fn abi_version() -> u32 {
                    $crate::ABI_VERSION
                }

                fn start() -> ::core::result::Result<(), ::std::string::String> {
                    let cx = $crate::Cx::new();
                    WIDGET.with(|w| $crate::Widget::start(&mut *w.borrow_mut(), &cx))
                }

                fn view() -> $crate::bindings::uniwidged::widget::types::Tree {
                    let cx = $crate::Cx::new();
                    let view = WIDGET.with(|w| $crate::Widget::view(&mut *w.borrow_mut(), &cx));
                    $crate::to_wire(&view)
                }

                fn next_wake() -> $crate::bindings::uniwidged::widget::types::Wake {
                    let wake = WIDGET.with(|w| $crate::Widget::wake(&*w.borrow()));
                    $crate::wake_to_wire(wake)
                }

                fn on_event(
                    event: $crate::bindings::uniwidged::widget::types::Event,
                ) -> bool {
                    let cx = $crate::Cx::new();
                    let event = $crate::event_from_wire(event);
                    WIDGET.with(|w| $crate::Widget::on_event(&mut *w.borrow_mut(), &event, &cx))
                }
            }

            $crate::bindings::export!(Component with_types_in $crate::bindings);
        };
    };
}

#[cfg(not(target_arch = "wasm32"))]
#[macro_export]
macro_rules! export {
    ($widget:ty) => {
        // Nothing to export off-target. The type is still checked by the trait
        // bound below, so a widget that does not implement `Widget` fails to
        // compile natively rather than only when someone builds it for WASM.
        const _: fn() = || {
            fn assert_widget<T: $crate::Widget>() {}
            assert_widget::<$widget>();
        };
    };
}

#[cfg(target_arch = "wasm32")]
pub(crate) mod convert;
#[cfg(target_arch = "wasm32")]
pub use convert::{event_from_wire, to_wire, wake_to_wire};
