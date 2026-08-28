//! Telling a running desktop that something changed.
//!
//! `uniwidged configure` writes a file; `uniwidged run` is a different process
//! that read it minutes ago. Until this existed the answer was "it takes effect
//! the next time the desktop starts", which is a sentence people read once and
//! then stop changing settings.
//!
//! ## Why a signal and a pid file
//!
//! Because the alternative is a socket, and a socket is a protocol: a wire
//! format, a version, a permission model, and a second thing to keep working.
//! What is actually being sent here is one bit — *look at your configuration
//! again* — and `SIGHUP` has meant exactly that to a daemon for forty years.
//! There is nothing to encode.
//!
//! It also costs the loop nothing. The handler writes one byte to the wake pipe
//! that already exists for finished tasks, so an idle desktop still blocks on
//! its window system's socket with no timer at all — see
//! `uniwidged_shell::wake`, and `ARCHITECTURE.md` §5 for why that matters.
//!
//! ## The dangerous part, and what is done about it
//!
//! `SIGHUP`'s default action is **terminate**. A pid file left behind by a
//! crash names a number the kernel will happily reuse, so signalling it blindly
//! is a way to kill a stranger's process — an editor, a shell, a build. So the
//! pid is checked twice before anything is sent: that the process exists, and
//! that it is *this* program, by reading `/proc/<pid>/comm`. A pid file whose
//! process is gone or is something else is stale, and stale means ignored
//! rather than signalled.
//!
//! There is no attempt to make the file atomic against a second desktop
//! starting. Two desktops on one configuration is already a mistake — they
//! would fight over placements — and this is not the layer that should be
//! detecting it.

#![cfg(unix)]

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// Set by the signal handler; read and cleared by the loop.
///
/// The handler does two things and both are async-signal-safe: a relaxed store
/// to this, and one non-blocking `write` to the wake pipe. Anything else — an
/// allocation, a lock, a `println!` — would be undefined behaviour in a
/// handler, and the kind that works until it does not.
static PENDING: AtomicBool = AtomicBool::new(false);

/// The wake pipe's write end, so the handler can reach it.
///
/// A raw descriptor in a static because a handler has no arguments and cannot
/// take a lock. `-1` means no pipe has been registered, which is the state
/// every process that is not `uniwidged run` stays in.
static WAKE_FD: AtomicI32 = AtomicI32::new(-1);

/// Where the running desktop records itself.
pub fn pid_file() -> PathBuf {
    crate::paths::keyring_dir().join("uniwidged.pid")
}

/// Claims the pid file and installs the handler.
///
/// Returns a guard that removes the file when the desktop exits. Not a
/// `Drop`-only type by accident: a desktop killed with `SIGKILL` leaves the
/// file behind, which is exactly why every reader treats a pid file as a
/// *claim* rather than a fact.
pub fn listen(wake_fd: std::os::fd::RawFd) -> io::Result<PidFile> {
    WAKE_FD.store(wake_fd, Ordering::Relaxed);
    // SAFETY: `handler` is async-signal-safe — a relaxed atomic store and one
    // non-blocking `write`, both on the permitted list. `SA_RESTART` so the
    // loop's `poll` is resumed rather than failing with `EINTR`, which is a
    // failure mode this code would otherwise have to handle everywhere.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as *const () as usize;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(libc::SIGHUP, &action, std::ptr::null_mut()) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    PidFile::claim(pid_file())
}

extern "C" fn handler(_signal: libc::c_int) {
    PENDING.store(true, Ordering::Relaxed);
    let fd = WAKE_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = [1u8];
        // SAFETY: one non-blocking write of one byte to a descriptor the loop
        // owns for its whole life. A full pipe is not an error — it means a
        // wake-up is already pending, and one is as good as ten.
        unsafe {
            libc::write(fd, byte.as_ptr().cast(), 1);
        }
    }
}

/// Whether a `SIGHUP` has arrived since this was last asked.
///
/// Swapped rather than read-then-cleared: a signal arriving between the two
/// would be lost, and the whole point is that a configuration change is never
/// silently dropped.
pub fn take_pending() -> bool {
    PENDING.swap(false, Ordering::Relaxed)
}

/// The pid file, removed when the desktop stops.
pub struct PidFile(PathBuf);

impl PidFile {
    fn claim(path: PathBuf) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, format!("{}\n", std::process::id()))?;
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        // Best effort. A desktop that could not remove its own pid file leaves
        // a stale one, and a stale one is what every reader already handles.
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Why a running desktop could not be reached.
#[derive(Debug, PartialEq, Eq)]
pub enum NotRunning {
    /// No pid file at all: nothing has ever claimed one here.
    NoPidFile,
    /// A pid file naming a process that is gone, or one that is not uniwidged.
    Stale(i32),
    /// A pid file that does not hold a pid.
    Unreadable(String),
}

impl std::fmt::Display for NotRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotRunning::NoPidFile => write!(f, "no desktop is running"),
            NotRunning::Stale(pid) => write!(
                f,
                "the desktop that wrote this configuration's pid file ({pid}) is gone"
            ),
            NotRunning::Unreadable(why) => write!(f, "{why}"),
        }
    }
}

/// The pid of the desktop running against this configuration, if there is one.
pub fn running_desktop(path: &Path) -> Result<i32, NotRunning> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(NotRunning::NoPidFile),
        Err(e) => return Err(NotRunning::Unreadable(format!("{}: {e}", path.display()))),
    };
    let pid: i32 = text
        .trim()
        .parse()
        .map_err(|_| NotRunning::Unreadable(format!("{} does not hold a pid", path.display())))?;
    if pid <= 1 {
        // Never plausible, and `1` in particular is init. Refusing here rather
        // than letting the checks below decide is the belt to their braces.
        return Err(NotRunning::Stale(pid));
    }
    if !is_uniwidged(pid) {
        return Err(NotRunning::Stale(pid));
    }
    Ok(pid)
}

/// Is this pid alive, and is it us?
///
/// Both halves matter. Liveness alone is not enough: pids are reused, and
/// `SIGHUP`'s default action is to terminate, so signalling a stranger on the
/// strength of a stale file is a way to kill somebody's editor.
///
/// The name check reads `/proc`, which is Linux. Where there is no `/proc` the
/// answer is liveness alone and this is documented as weaker rather than
/// pretended to be the same — and the platforms without `/proc` are the ones
/// that have no desktop shell here yet either.
fn is_uniwidged(pid: i32) -> bool {
    // SAFETY: signal 0 sends nothing; it only performs the permission and
    // existence checks. This is the documented way to ask whether a pid is
    // alive without touching it.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    match std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        Ok(comm) => comm.trim() == "uniwidged",
        // No `/proc`. Liveness is all there is, and it is what this platform
        // can honestly offer.
        Err(_) => !cfg!(target_os = "linux"),
    }
}

/// Asks the running desktop to read its configuration again.
///
/// Quiet about *not* finding one: every command that changes the configuration
/// calls this, and most of the time no desktop is running. A message every time
/// somebody ran `uniwidged configure` on a server would be noise, and noise is
/// how a real message gets missed.
pub fn nudge(path: &Path) -> Result<i32, NotRunning> {
    let pid = running_desktop(path)?;
    // SAFETY: a pid this function has just checked is alive and is uniwidged.
    if unsafe { libc::kill(pid, libc::SIGHUP) } != 0 {
        return Err(NotRunning::Unreadable(
            io::Error::last_os_error().to_string(),
        ));
    }
    Ok(pid)
}

/// `uniwidged reload` — what a script runs after editing the file by hand.
pub fn command(_args: &[String]) -> Result<(), String> {
    match nudge(&pid_file()) {
        Ok(pid) => {
            println!("  asked the desktop ({pid}) to read its configuration again");
            Ok(())
        }
        Err(why) => Err(format!(
            "{why}. A configuration is read at start-up, so there is nothing \
             to tell."
        )),
    }
}

/// Called by every command that writes the configuration.
///
/// Prints one line when a desktop was told, and nothing when there was none.
pub fn nudge_quietly() {
    if let Ok(pid) = nudge(&pid_file()) {
        println!("  the running desktop ({pid}) has been told");
    }
}

#[cfg(test)]
mod tests;
