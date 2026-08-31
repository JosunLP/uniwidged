//! The host, through the component's imports.
//!
//! A thin, typed skin over the generated bindings, so a widget author writes
//! `cx.fetch(url)` rather than a module path four levels deep — and so the
//! error type is one they can match on.

use crate::bindings::uniwidged::widget::host as raw;
use uniwidged_view::{ImageHandle, Role};

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

/// One of your widget's own settings, as your manifest declared it and your
/// user set it.
///
/// There is no `Choice` variant: a choice's value *is* text, and the set it
/// has to come from is your manifest's business. The host has already checked
/// it, so matching on the strings you listed needs no arm for one you did not.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Text(String),
    Number(f64),
    Toggle(bool),
}

impl SettingValue {
    /// The text of a `text` or `choice` setting, or `None` for the others.
    ///
    /// A convenience for the common shape — `matches!` on a variant you
    /// declared is the honest read, and this is what saves writing it for a
    /// choice you are about to compare against a literal.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            SettingValue::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            SettingValue::Number(number) => Some(*number),
            _ => None,
        }
    }

    pub fn as_toggle(&self) -> Option<bool> {
        match self {
            SettingValue::Toggle(toggle) => Some(*toggle),
            _ => None,
        }
    }
}

impl From<raw::SettingValue> for SettingValue {
    fn from(value: raw::SettingValue) -> Self {
        match value {
            raw::SettingValue::Text(text) => SettingValue::Text(text),
            raw::SettingValue::Number(number) => SettingValue::Number(number),
            raw::SettingValue::Toggle(toggle) => SettingValue::Toggle(toggle),
        }
    }
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

    /// Reads one of your widget's own secrets from the OS keychain.
    ///
    /// `Ok(None)` means you never set that key — what your first run finds,
    /// and not an error, for the same reason [`Cx::read_file`] answers the
    /// same way. A key is at most 64 characters of `[A-Za-z0-9._-]`; anything
    /// else is [`Denied::Malformed`].
    ///
    /// Nothing here can name another widget's secrets: the namespace is the
    /// handle's, not an argument.
    pub fn get_secret(&self, key: &str) -> Result<Option<Vec<u8>>, Denied> {
        raw::get_secret(key).map_err(Into::into)
    }

    /// Stores one, replacing whatever was there. At most 64 KiB, and at most
    /// 64 keys per widget — both [`Denied::Malformed`] past that, because both
    /// are facts about the request rather than about the host.
    pub fn set_secret(&self, key: &str, value: &[u8]) -> Result<(), Denied> {
        raw::set_secret(key, value).map_err(Into::into)
    }

    /// Forgets one. `Ok(false)` means there was nothing to forget, so signing
    /// out twice is not something to guard against.
    pub fn remove_secret(&self, key: &str) -> Result<bool, Denied> {
        raw::remove_secret(key).map_err(Into::into)
    }

    /// Reads one of your widget's own settings.
    ///
    /// **No capability needed.** These are the settings your own manifest
    /// declared, and nothing here can name another widget's.
    ///
    /// `None` means your manifest never declared that key, which is a bug in
    /// your widget rather than a state to handle: a declared key always has a
    /// value, because an unset one is its declared default. The type is always
    /// the one you declared — the host refuses anything else before you see
    /// it — so matching on the variant you expect needs no other arm.
    ///
    /// ```ignore
    /// let warn_at = match cx.setting("warn-at") {
    ///     Some(SettingValue::Number(n)) => n,
    ///     // Only reachable if the manifest and this line disagree.
    ///     _ => 0.8,
    /// };
    /// ```
    ///
    /// When the user changes one, you are sent [`Event::Configured`]; read
    /// them again and return `true` if your view would differ.
    ///
    /// [`Event::Configured`]: crate::Event::Configured
    pub fn setting(&self, key: &str) -> Option<SettingValue> {
        raw::setting(key).map(Into::into)
    }

    /// How busy the machine's processors are, `0.0` to `1.0`, over the window
    /// since this was last asked. Never blocks; two calls within 250 ms get
    /// the same answer rather than noise.
    pub fn cpu_load(&self) -> Result<f32, Denied> {
        raw::cpu_load().map_err(Into::into)
    }

    /// How much of the machine's memory is in use, `0.0` to `1.0`.
    ///
    /// "In use" is what the kernel could *not* hand to a new allocation
    /// without evicting something — not "not free", which on a machine that
    /// has been up for an hour is nearly all of it and means nothing.
    pub fn memory_load(&self) -> Result<f32, Denied> {
        raw::memory_load().map_err(Into::into)
    }

    /// Goes to the host's log, prefixed with your widget's name.
    ///
    /// Always available: a widget that cannot say what went wrong is one
    /// nobody can help.
    pub fn log(&self, message: &str) {
        raw::log(message);
    }

    /// Hands the host a PNG and gets back the handle an `image` node names.
    ///
    /// **No capability needed** — the bytes came from you and go to your own
    /// surface. What is bounded is the host's memory: at most 1 MiB encoded
    /// and 1024 pixels a side ([`Denied::Malformed`]), and at most sixteen
    /// held at once ([`Denied::Unavailable`] — release one first).
    ///
    /// Decoded once, here. Register in `start` and name the handle in every
    /// view; a restarted widget registers again, because a handle belongs to
    /// the instance that made it.
    pub fn register_image(&self, png: &[u8]) -> Result<ImageHandle, Denied> {
        raw::register_image(png)
            .map(ImageHandle)
            .map_err(Into::into)
    }

    /// Forgets an image. `false` when there was nothing to forget.
    pub fn release_image(&self, handle: ImageHandle) -> bool {
        raw::release_image(handle.0)
    }
}
