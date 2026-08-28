//! The host, through the component's imports.
//!
//! A thin, typed skin over the generated bindings, so a widget author writes
//! `cx.fetch(url)` rather than a module path four levels deep — and so the
//! error type is one they can match on.

use crate::bindings::uniwidged::widget::host as raw;
use uniwidged_view::Role;

/// What a finished command left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// The exit code, or -1 where it was killed — by its own timeout, or by a
    /// signal. Zero is the only value that conventionally means success.
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    /// `stdout` as text with the trailing newline removed, which is what a
    /// widget almost always wants and never wants to write out.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim_end().to_string()
    }

    pub fn succeeded(&self) -> bool {
        self.status == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// The answer to one asynchronous request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskResult {
    Command(CommandOutput),
    Http(HttpResponse),
    /// The work did not happen — the command is not installed, the request
    /// timed out. Distinct from a non-zero exit status, which *is* a result:
    /// show "could not run it" differently from "it said no".
    Failed(String),
}

impl From<raw::TaskResult> for TaskResult {
    fn from(result: raw::TaskResult) -> Self {
        match result {
            raw::TaskResult::Command(output) => TaskResult::Command(CommandOutput {
                status: output.status,
                stdout: output.stdout,
                stderr: output.stderr,
            }),
            raw::TaskResult::Http(response) => TaskResult::Http(HttpResponse {
                status: response.status,
                body: response.body,
            }),
            raw::TaskResult::Failed(why) => TaskResult::Failed(why),
        }
    }
}

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

    /// How wide `text` would be in `role`, in logical pixels.
    ///
    /// The number the host's own layout will arrive at, in the theme it is
    /// about to draw in — so a widget can decide for itself whether a label
    /// fits, rather than against a guess. You still do not choose the font,
    /// the size or the weight; this is a window onto what the host chose.
    pub fn measure_text(&self, text: &str, role: Role) -> Result<f32, Denied> {
        raw::measure_text(text, crate::convert::role(role)).map_err(Into::into)
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

    /// Runs a command your manifest declared.
    ///
    /// Returns a token immediately; the answer arrives as
    /// [`Event::Delivered`](crate::Event::Delivered) carrying it. There is
    /// deliberately no version that waits — a widget waiting on a process is a
    /// widget holding its deadline open, and the host would interrupt it.
    ///
    /// The command is run without a shell, so nothing in `args` can become
    /// another command, and with almost no environment.
    pub fn run_command(&self, name: &str, args: &[&str]) -> Result<u32, Denied> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        raw::run_command(name, &args).map_err(Into::into)
    }

    /// Collects a finished request, by the token that started it.
    ///
    /// Call it when the `delivered` event for that token arrives. Asking
    /// earlier answers [`Denied::Unavailable`]; asking for a token that is not
    /// yours answers [`Denied::Malformed`].
    pub fn take_result(&self, token: u32) -> Result<TaskResult, Denied> {
        raw::take_result(token).map(Into::into).map_err(Into::into)
    }

    /// Reads a file from your widget's own directory.
    ///
    /// `Ok(None)` means it is not there, which is what your first run finds
    /// and is not an error. Everything else is: a path outside your directory
    /// is [`Denied::Malformed`], and a host with no storage for you is
    /// [`Denied::Unavailable`].
    pub fn read_file(&self, path: &str) -> Result<Option<Vec<u8>>, Denied> {
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
