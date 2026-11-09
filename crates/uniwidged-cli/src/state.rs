//! What is on the screen right now.
//!
//! `uniwidged list` reads the configuration and answers *what is installed*.
//! That is a different question from *what is running*, and until this file the
//! second one had no answer at all: a widget can be enabled in the file and not
//! on the screen because its component was deleted, because its manifest no
//! longer parses, or because the supervisor gave up on it after three failures.
//! The desktop knew all three and said so on a terminal nobody was attached to.
//!
//! ## A file, not a socket
//!
//! For the same reason `daemon` sends a signal rather than opening one: a
//! socket is a wire format, a version and a permission model, and what is being
//! answered here is a question about a dozen short strings that change a few
//! times a day. The desktop writes them whenever the set changes; anybody who
//! wants to know reads the file.
//!
//! It is written the way the `fs` capability writes: to a temporary file and
//! then renamed, so a reader never sees half a document. That matters more here
//! than it looks — `uniwidged status` is most likely to be run at exactly the
//! moment something is changing.
//!
//! ## Stale is the normal case, and is handled the same way as the pid file
//!
//! A desktop killed with `SIGKILL` leaves this behind, exactly as it leaves its
//! pid file behind. So this file is **never trusted on its own**: every reader
//! asks [`crate::daemon::running_desktop`] first, and a state file whose
//! desktop is gone is ignored rather than reported. The pid inside it is
//! checked against that answer too, so a state file left by an older desktop
//! cannot be read as the current one's.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where the running desktop records what it is showing.
pub fn path() -> PathBuf {
    crate::paths::state_dir().join("uniwidged.state")
}

/// What a widget is doing, as the supervisor sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Health {
    /// Started, drawing, and answering.
    Running,
    /// It failed and is being given another instance.
    Restarting,
    /// Three failures. Its last frame stays on screen and nothing runs.
    GivenUp,
}

impl std::fmt::Display for Health {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Health::Running => write!(f, "running"),
            Health::Restarting => write!(f, "restarting"),
            Health::GivenUp => write!(f, "given up on"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetState {
    pub name: String,
    pub version: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default)]
    pub locked: bool,
    pub health: Health,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct State {
    /// The desktop that wrote it, checked against the pid file before any of
    /// the rest is believed.
    pub pid: i32,
    /// When it started, in seconds since the epoch.
    pub since: i64,
    /// `wlr-layer-shell`, `xdg_toplevel (no layer shell)`, the X11 wording —
    /// whatever [`uniwidged_shell::platform::desktop::DesktopWindow::describe`]
    /// answered. The single most useful line when a widget is not where it
    /// should be, because it is the one that says whether this session can put
    /// it there at all.
    #[serde(default)]
    pub shell: String,
    #[serde(default, rename = "widget")]
    pub widgets: Vec<WidgetState>,
}

impl State {
    /// Writes it where [`path`] says, atomically.
    ///
    /// Best effort and never fatal: a desktop that cannot write this still
    /// draws, and the only thing lost is an answer to a question nobody may
    /// ask. It is not worth a line on the terminal either — the failure would
    /// repeat on every change.
    pub fn write(&self) {
        self.write_to(&path());
    }

    /// [`write`](Self::write), to a path the caller names.
    pub fn write_to(&self, path: &Path) {
        let Some(parent) = path.parent() else { return };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(text) = toml::to_string_pretty(self) else {
            return;
        };
        // Named for this process, so two desktops writing at once cannot land
        // on one temporary file and rename each other's half.
        let temporary = parent.join(format!("uniwidged.state.{}", std::process::id()));
        if std::fs::write(&temporary, text).is_ok() && std::fs::rename(&temporary, path).is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
    }

    /// Reads one, having been told which desktop is running.
    ///
    /// The half of [`current`] that does not depend on the process's own
    /// configuration, so that it can be tested against a file a test wrote.
    pub fn read_for(path: &Path, pid: i32) -> Result<Self, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // A desktop from before this file existed, or one that could
                // not write it. It is running; there is simply nothing to read.
                return Err(format!(
                    "a desktop ({pid}) is running but has written no state file"
                ));
            }
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let state: State = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if state.pid != pid {
            // Left by an earlier desktop that was killed, with a new one now
            // running. Reporting the old one's widgets as the new one's would
            // be worse than reporting nothing.
            return Err(format!(
                "the state file names desktop {} and the pid file names {pid}; \
                 one of them is stale",
                state.pid
            ));
        }
        Ok(state)
    }
}

/// Removes the state file when the desktop returns from `run`.
///
/// A guard rather than a call at the end, for the reason the pid file is one:
/// `run` returns from several places and one of them is an error, and a state
/// file left behind by a desktop that stopped would claim widgets are on a
/// screen that is empty. A desktop killed with `SIGKILL` still leaves it —
/// which is why [`current`] checks the pid file first and does not trust this
/// on its own.
pub struct Written;

impl Drop for Written {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(path());
    }
}

/// What the desktop running against this configuration is showing.
///
/// `Ok(None)` means no desktop is running, which is the common case and not an
/// error. `Err` is a desktop that is running and whose state file could not be
/// read — which is worth saying, because it means `status` cannot answer a
/// question that does have an answer.
pub fn current() -> Result<Option<State>, String> {
    let pid = match crate::daemon::running_desktop(&crate::daemon::pid_file()) {
        Ok(pid) => pid,
        Err(_) => return Ok(None),
    };
    State::read_for(&path(), pid).map(Some)
}

#[cfg(test)]
mod tests;
