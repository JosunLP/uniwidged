//! The sandbox a widget runs in.
//!
//! A widget is a WASM component. WASI Preview 2 gives it no ambient authority
//! at all, so the interesting question is not what to forbid but what to hand
//! it — and this crate is the answer to that, plus the two limits that stop a
//! widget hurting the desktop even with nothing granted.
//!
//! ## Capabilities are host-scoped, not category-scoped
//!
//! `net:api.open-meteo.com` rather than `network`. The difference is whether a
//! user can meaningfully consent: "this weather widget talks to
//! api.open-meteo.com" is a sentence somebody can agree or object to, and
//! "this widget uses the network" is not.
//!
//! ## Containment
//!
//! * **Memory** — a `ResourceLimiter` cap per instance, declared in the
//!   manifest and shown to the user before installation.
//! * **Time** — epoch-based interruption with a deadline on every call. A
//!   widget that blows it is suspended, marked errored, and keeps showing its
//!   last frame. **The desktop never blocks on guest code**, which is the one
//!   invariant that makes running other people's code on a desktop reasonable.
//! * **State** — its own data directory, its own keychain namespace, and no
//!   path out of either. The directory is [`services::DataDir`], and it
//!   checks containment twice: once textually, and once more after the
//!   filesystem has resolved the path, which is the only check that can see a
//!   symlink.
//!
//! ## Status
//!
//! [`Capability`] and [`Policy`] are complete and are the security boundary,
//! so they are tested for what they *refuse*. [`Services`] is what is behind
//! the gate, and every one of them now has something there — the last, and
//! the one it was right to leave until last, is [`keyring`]. An embedder that
//! supplies nothing still gets the whole sandbox: an ungranted capability
//! answers `not-granted` and a granted one this host cannot serve answers
//! `unavailable`, which is a different fact and one a widget can act on.

#![allow(dead_code)]

pub mod capability;
pub mod keyring;
pub mod limits;
pub mod manifest;
pub mod net;
pub mod services;
pub mod signature;
pub mod tasks;

#[cfg(feature = "wasmtime")]
pub mod instance;

pub use capability::{Capability, Policy, Request};
pub use keyring::{FileKeyring, Keyring, MemoryKeyring, SecretStore};
pub use limits::Limits;
pub use manifest::{ABI_VERSION, Manifest, ManifestError};
pub use net::Http;
pub use services::{DataDir, Locale, Services};
pub use signature::{PublicKey, SecretKey, SignatureFile, TrustedKeys};
pub use tasks::{TaskResult, Tasks, Waker};
