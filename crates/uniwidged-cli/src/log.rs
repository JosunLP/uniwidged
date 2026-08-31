//! What the desktop said, after the terminal has gone.
//!
//! `uniwidged run` prints sixty different lines: which shell it got, where each
//! widget was placed, a driver with no transparent swapchain, a setting in the
//! file that a widget will not accept, a link that was not opened and why, a
//! widget given up on after three failures. Every one of them is a thing
//! somebody needs at the moment they are wondering why their desktop looks
//! wrong.
//!
//! And almost nobody ever sees one, because the normal way a widget host runs
//! is **started with the session**. `uniwidged autostart` writes a `.desktop`
//! entry, the session manager launches it with no terminal attached, and
//! standard output goes to the session journal if the user is lucky and to
//! `/dev/null` if they are not. A diagnostic nobody can read is not a
//! diagnostic.
//!
//! So the desktop keeps a log. It is deliberately the smallest thing that
//! solves that problem and nothing more.
//!
//! ## Why not a logging framework
//!
//! `tracing` was in the workspace manifest from the first commit and was used
//! by nothing — which is worth saying plainly rather than quietly deleting,
//! because it is the shape of decision this file is about. A framework buys
//! levels, targets, spans, filtering and a subscriber to configure, and this
//! program has one process, one thread that logs, and one question to answer:
//! *what did it say, and when*. The entry has been removed.
//!
//! What is here instead is a hundred lines: an append-only file, a timestamp,
//! a size cap and one rotation.
//!
//! ## One rotation, not five
//!
//! A desktop's log is for the last thing that went wrong, not for a history.
//! Past the cap the current file becomes `uniwidged.log.1`, replacing whatever
//! was there, and a fresh one is started — so the disk holds at most twice the
//! cap however long the desktop runs, and the previous file is still there when
//! the interesting line was written just before a rotation.
//!
//! ## It never fails
//!
//! Nothing here returns an error to its caller and nothing here panics. A log
//! that can stop a desktop is worse than no log: the disk being full is already
//! somebody's bad day, and a widget host taking their clock off the screen over
//! it would be this program's contribution to it. A write that fails is
//! dropped, and the terminal — if there is one — still has the line.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// How large the log may get before it is rotated.
///
/// A megabyte is a few thousand lines, which on a desktop that says something
/// only when something happens is months. Small enough that nobody has to think
/// about it, large enough that a widget in a restart loop does not push the
/// interesting line out within the minute.
const CAP: u64 = 1024 * 1024;

/// Where the desktop writes what it said.
///
/// Beside the configuration, like everything else this program keeps, and
/// therefore following `UNIWIDGED_CONFIG` — so a test desktop, or two desktops
/// running side by side, do not write into each other's.
pub fn path() -> PathBuf {
    crate::paths::state_dir().join("uniwidged.log")
}

/// The previous file, kept across exactly one rotation.
pub fn previous_path() -> PathBuf {
    rotated(&path())
}

/// `uniwidged.log` → `uniwidged.log.1`.
fn rotated(path: &Path) -> PathBuf {
    let mut rotated = path.as_os_str().to_owned();
    rotated.push(".1");
    PathBuf::from(rotated)
}

struct Log {
    /// Where it writes. Carried rather than asked for again on every
    /// rotation, so that a rename lands on the file that was opened — and so
    /// that a test can hand one a scratch path without touching the
    /// environment every other test in this binary reads.
    path: PathBuf,
    file: Option<std::fs::File>,
    written: u64,
    locale: uniwidged_runtime::services::Locale,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

/// Opens the log for the process's lifetime.
///
/// Called once, by `uniwidged run`. Every other command prints to a terminal
/// somebody is looking at, and writing their output to a file would be filling
/// a log with the answers to questions that were already answered.
///
/// Returns where it is writing, or why it is not. A desktop whose log could not
/// be opened still runs; it says so on the terminal it may not have, which is
/// the best that can be done about it.
pub fn open() -> Result<PathBuf, String> {
    let path = path();
    // Once, at start-up. A desktop that is running when the clock changes
    // over daylight saving writes an hour of lines against the old offset,
    // which is a smaller wrong than asking the C library for the offset on
    // every line.
    let log = Log::open_at(path.clone(), uniwidged_runtime::services::Locale::system())?;
    // A second `open` would be a second desktop in one process, which cannot
    // happen — but `set` rather than `get_or_init` so that if it ever did, the
    // first one keeps its file rather than silently sharing.
    let _ = LOG.set(Mutex::new(log));
    Ok(path)
}

/// Writes one line, to the terminal and to the log.
///
/// `trouble` decides which stream the terminal copy goes to, and nothing else:
/// there are no levels in the file. A log with levels is a log with a filter,
/// and a filter is a way to have thrown away the line somebody needed.
pub fn line(trouble: bool, args: std::fmt::Arguments<'_>) {
    let text = std::fmt::format(args);
    if trouble {
        eprintln!("uniwidged: {text}");
    } else {
        println!("{text}");
    }
    let Some(log) = LOG.get() else {
        return;
    };
    // A poisoned mutex means a previous writer panicked while holding it.
    // Nothing here can panic, so this is unreachable — and recovering rather
    // than unwrapping keeps a log from being the thing that ends a desktop.
    let mut log = match log.lock() {
        Ok(log) => log,
        Err(poisoned) => poisoned.into_inner(),
    };
    log.write(trouble, &text);
}

impl Log {
    fn open_at(path: PathBuf, locale: uniwidged_runtime::services::Locale) -> Result<Self, String> {
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return Err(format!("{}: {e}", parent.display()));
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let written = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Log {
            path,
            file: Some(file),
            written,
            locale,
        })
    }

    /// The previous file, kept across exactly one rotation.
    fn previous(&self) -> PathBuf {
        rotated(&self.path)
    }

    fn write(&mut self, trouble: bool, text: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (year, month, day, hour, minute, second) = self.locale.civil(now);
        // Fixed width and sortable, and deliberately not `Locale::format_time`:
        // that one omits seconds because a widget showing them asks to be woken
        // sixty times as often, and a log line is the one place where the
        // second is the point. The date order is this file's own for the same
        // reason — a log is read by whoever is debugging it, and `08/09` is
        // ambiguous to half of them.
        let line = format!(
            "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} {}{text}\n",
            if trouble { "! " } else { "  " }
        );
        // Rotated before the write rather than after, so the cap is a ceiling
        // rather than a ceiling plus one line.
        if self.written + line.len() as u64 > CAP {
            self.rotate();
        }
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if file.write_all(line.as_bytes()).is_ok() {
            self.written += line.len() as u64;
        }
    }

    /// Moves the current file aside and starts a new one.
    ///
    /// A failure anywhere here leaves `self.file` as it was and the log keeps
    /// growing past the cap, which is the correct order of preference: a log
    /// that is too big is a nuisance, and a desktop that stopped logging
    /// because a rename failed is a mystery.
    fn rotate(&mut self) {
        // Dropped first: renaming an open file works on Unix, but the handle
        // would keep writing into the *renamed* file, and the new one would
        // stay empty for as long as the desktop ran.
        self.file = None;
        if std::fs::rename(&self.path, self.previous()).is_err() {
            self.file = std::fs::OpenOptions::new()
                .append(true)
                .open(&self.path)
                .ok();
            return;
        }
        self.file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok();
        self.written = 0;
    }
}

/// Everything the desktop says, on the terminal and in the log.
macro_rules! say {
    ($($arg:tt)*) => { $crate::log::line(false, format_args!($($arg)*)) };
}

/// The same, for something that went wrong. Prefixed `uniwidged:` on the
/// terminal and `!` in the file, so a log can be skimmed for the bad lines.
///
/// Not called `warn`: that is a built-in attribute macro, and a `macro_rules!`
/// of the same name is ambiguous at every call site rather than at its
/// definition — which the compiler reports as an error about the *use*.
macro_rules! problem {
    ($($arg:tt)*) => { $crate::log::line(true, format_args!($($arg)*)) };
}

pub(crate) use {problem, say};

/// `uniwidged log [-n <lines>] [-f]` — the last of what a desktop said.
///
/// Reads the previous file too when the current one is shorter than asked for,
/// because the line somebody is looking for is most likely the one written just
/// before a rotation.
///
/// `-f` keeps reading, the way `tail -f` does, which is how a desktop started
/// with the session is watched from a terminal it never had. It polls rather
/// than watching the file: an inotify handle is a platform, and what is being
/// waited for is a line a human will read, for which a quarter of a second is
/// instant. It ends with the terminal — Ctrl-C — or when the desktop it was
/// following stops, which is worth saying rather than sitting on a file that
/// will not change.
pub fn command(args: &[String]) -> Result<(), String> {
    let wanted = match args.iter().position(|a| a == "-n" || a == "--lines") {
        Some(at) => args
            .get(at + 1)
            .ok_or("how many lines? `uniwidged log -n <lines>`")?
            .parse::<usize>()
            .map_err(|_| "`-n` takes a number".to_string())?,
        None => 40,
    };
    let follow = args.iter().any(|a| a == "-f" || a == "--follow");

    let current = path();
    let mut lines = read_lines(&current);
    if lines.len() < wanted {
        let mut earlier = read_lines(&previous_path());
        earlier.extend(lines);
        lines = earlier;
    }
    if lines.is_empty() && !follow {
        println!(
            "  nothing logged yet ({}). A desktop writes here while it runs.",
            current.display()
        );
        return Ok(());
    }
    println!("  {}\n", current.display());
    for line in lines.iter().skip(lines.len().saturating_sub(wanted)) {
        println!("{line}");
    }
    if follow {
        follow_lines(&current);
    }
    Ok(())
}

/// Prints each line appended to `path` as it arrives.
///
/// Returns when the desktop is gone — checked through the pid file, the same
/// way every other command finds it — or when a rotation moves the file
/// underneath the reader, which restarts from the top of the new one. A
/// desktop that was never running is followed anyway: the usual reason to
/// run this is to watch one start.
fn follow_lines(path: &Path) {
    use std::io::{BufRead, Seek};

    let mut offset = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut was_running = false;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        #[cfg(unix)]
        let running = crate::daemon::running_desktop(&crate::daemon::pid_file()).is_ok();
        #[cfg(not(unix))]
        let running = false;
        if was_running && !running {
            println!("  the desktop has stopped");
            return;
        }
        was_running |= running;

        let Ok(mut file) = std::fs::File::open(path) else {
            continue;
        };
        let length = file.metadata().map(|m| m.len()).unwrap_or(0);
        if length < offset {
            // Rotated: the file is a new one, and everything in it is new.
            offset = 0;
        }
        if length == offset {
            continue;
        }
        if file.seek(std::io::SeekFrom::Start(offset)).is_err() {
            continue;
        }
        let mut reader = std::io::BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                // A partial line — the desktop is mid-write — is left for the
                // next turn rather than printed in two halves.
                Ok(0) => break,
                Ok(n) if line.ends_with('\n') => {
                    print!("{line}");
                    offset += n as u64;
                }
                Ok(_) => break,
                Err(_) => break,
            }
        }
        let _ = std::io::stdout().flush();
    }
}

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|text| text.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
