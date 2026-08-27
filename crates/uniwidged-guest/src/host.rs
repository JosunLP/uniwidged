//! The host, through the component's imports.
//!
//! A thin, typed skin over the generated bindings, so a widget author writes
//! `cx.fetch(url)` rather than a module path four levels deep — and so the
//! error type is one they can match on.

use crate::bindings::uniwidged::widget::host as raw;

/// Why a request was refused.
///
/// Three cases, and the difference matters: `NotGranted` means the manifest
/// never asked, so the fix is the manifest. `Unavailable` means it did ask and
/// the user agreed, but this host cannot serve it — which a widget should
/// report differently, because there is nothing its author can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denied {
    NotGranted,
    Malformed(String),
    Unavailable(String),
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Denied::NotGranted => write!(f, "not granted"),
            Denied::Malformed(why) => write!(f, "malformed: {why}"),
            Denied::Unavailable(why) => write!(f, "unavailable: {why}"),
        }
    }
}

impl From<raw::Denied> for Denied {
    fn from(denied: raw::Denied) -> Self {
        match denied {
            raw::Denied::NotGranted => Denied::NotGranted,
            raw::Denied::Malformed(why) => Denied::Malformed(why),
            raw::Denied::Unavailable(why) => Denied::Unavailable(why),
        }
    }
}

/// The host, as a widget sees it.
///
/// Every method is backed by a capability the manifest declared and the user
/// approved. Calling one you were not granted returns [`Denied::NotGranted`]
/// rather than trapping, so a widget can degrade — a weather widget without
/// `net` should show its last reading and say it is stale, not disappear.
#[derive(Debug, Default)]
pub struct Cx {
    _private: (),
}

impl Cx {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn format_time(&self, epoch_seconds: i64) -> Result<String, Denied> {
        raw::format_time(epoch_seconds).map_err(Into::into)
    }

    pub fn format_date(&self, epoch_seconds: i64) -> Result<String, Denied> {
        raw::format_date(epoch_seconds).map_err(Into::into)
    }

    pub fn measure_text(&self, text: &str, role: u8) -> Result<f32, Denied> {
        raw::measure_text(text, role).map_err(Into::into)
    }

    /// Starts an HTTPS request. The answer arrives as
    /// [`Event::Delivered`](crate::Event::Delivered) carrying this token.
    ///
    /// It does not block, and there is deliberately no version that does: a
    /// widget blocked on the network is a widget holding its deadline open,
    /// and the host would interrupt it.
    pub fn fetch(&self, url: &str) -> Result<u32, Denied> {
        raw::fetch(url).map_err(Into::into)
    }

    pub fn take_response(&self, token: u32) -> Result<Vec<u8>, Denied> {
        raw::take_response(token).map_err(Into::into)
    }

    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, Denied> {
        raw::read_file(path).map_err(Into::into)
    }

    pub fn write_file(&self, path: &str, bytes: &[u8]) -> Result<(), Denied> {
        raw::write_file(path, bytes).map_err(Into::into)
    }

    pub fn get_secret(&self, key: &str) -> Result<Vec<u8>, Denied> {
        raw::get_secret(key).map_err(Into::into)
    }

    pub fn set_secret(&self, key: &str, value: &[u8]) -> Result<(), Denied> {
        raw::set_secret(key, value).map_err(Into::into)
    }

    pub fn cpu_load(&self) -> Result<f32, Denied> {
        raw::cpu_load().map_err(Into::into)
    }

    /// Goes to the host's log, prefixed with your widget's name.
    ///
    /// Always available: a widget that cannot say what went wrong is one
    /// nobody can help.
    pub fn log(&self, message: &str) {
        raw::log(message);
    }
}
