//! The log, and the two things it has to get right.
//!
//! The rotation, because a file that grows without bound on a desktop that runs
//! for months is a bug somebody discovers with a full disk. And the timestamp
//! format, because a log's whole value is being able to line one of its lines up
//! against something that happened.
//!
//! `Log` is exercised directly, against a path each test owns, rather than
//! through the global: the global is a `OnceLock` set by `uniwidged run`, and
//! its path comes from `UNIWIDGED_CONFIG` — a process-wide variable that a test
//! setting it would be setting for every other test running beside it.

use super::*;

/// A directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "uniwidged-log-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Scratch(dir)
    }

    fn log(&self) -> PathBuf {
        self.0.join("uniwidged.log")
    }

    fn fresh(&self) -> Log {
        Log::open_at(self.log(), uniwidged_runtime::services::Locale::default()).expect("the log")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_line_carries_the_time_it_was_written_and_what_was_said() {
    let scratch = Scratch::new("format");
    let mut log = scratch.fresh();
    log.write(false, "clock at 40,40 240x160");
    log.write(true, "could not move `clock`");

    let text = std::fs::read_to_string(scratch.log()).expect("the log");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");

    // `YYYY-MM-DD HH:MM:SS` and then the line. Fixed width so that a log is
    // sortable and skimmable, and with seconds — which `Locale::format_time`
    // deliberately omits, and which are the point here.
    let (stamp, rest) = lines[0].split_at(19);
    assert_eq!(stamp.len(), 19);
    assert!(
        stamp.chars().nth(4) == Some('-') && stamp.chars().nth(13) == Some(':'),
        "{stamp}"
    );
    assert_eq!(rest, "   clock at 40,40 240x160");

    // And the one that went wrong is marked, so a long log can be skimmed for
    // the bad lines without reading any of the good ones.
    assert!(
        lines[1].contains(" ! could not move `clock`"),
        "{}",
        lines[1]
    );
}

#[test]
fn opening_an_existing_log_appends_rather_than_truncating() {
    // A desktop restarted by the session — a crash, a log-out — must not
    // erase the lines that say why it went. Which is also why the counter
    // starts at the file's current size: the cap is on the file, not on this
    // process's share of it.
    let scratch = Scratch::new("append");
    scratch.fresh().write(false, "before the restart");
    let mut again = scratch.fresh();
    assert!(again.written > 0, "the size of what was there is the start");
    again.write(false, "after it");
    let text = std::fs::read_to_string(scratch.log()).expect("the log");
    assert!(
        text.contains("before the restart") && text.contains("after it"),
        "{text}"
    );
}

#[test]
fn the_log_is_rotated_rather_than_growing_without_bound() {
    // The property a desktop that runs for months depends on: at most twice the
    // cap on disk, however much is written.
    let scratch = Scratch::new("rotate");
    let mut log = scratch.fresh();
    // Comfortably past the cap, in lines of a plausible length.
    let line = "a".repeat(200);
    let each = 200 + 22;
    let enough = (CAP as usize / each) + 50;
    for _ in 0..enough {
        log.write(false, &line);
    }

    let current = std::fs::metadata(scratch.log()).expect("the log").len();
    let previous = std::fs::metadata(rotated(&scratch.log()))
        .expect("the rotated log")
        .len();
    assert!(
        current <= CAP,
        "the current file is over the cap: {current}"
    );
    assert!(
        previous <= CAP,
        "the rotated file is over the cap: {previous}"
    );
    // And nothing else was left behind: one rotation, not a numbered series
    // nobody ever deletes.
    let files: Vec<_> = std::fs::read_dir(&scratch.0)
        .expect("the directory")
        .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
        .filter(|name| name.starts_with("uniwidged.log"))
        .collect();
    assert_eq!(files.len(), 2, "{files:?}");
}

#[test]
fn what_was_written_just_before_a_rotation_is_still_readable() {
    // The reason there is a previous file at all. The interesting line is
    // usually the last one before something went wrong, and a rotation that
    // discarded it would throw away exactly the line somebody came for.
    let scratch = Scratch::new("previous");
    let mut log = scratch.fresh();
    log.write(true, "the line that mattered");
    let line = "a".repeat(200);
    for _ in 0..((CAP as usize / 222) + 10) {
        log.write(false, &line);
    }
    let earlier = std::fs::read_to_string(rotated(&scratch.log())).expect("the rotated log");
    assert!(
        earlier.contains("the line that mattered"),
        "the rotation discarded the first line"
    );
}

#[test]
fn a_log_that_cannot_be_written_is_not_an_error_anybody_hears_about() {
    // The rule the module note states: nothing here may stop a desktop. A log
    // whose file has gone — a tmpfs cleared underneath it, a disk unmounted —
    // drops the line and carries on.
    let scratch = Scratch::new("gone");
    let mut log = Log {
        path: scratch.log(),
        file: None,
        written: 0,
        locale: uniwidged_runtime::services::Locale::default(),
    };
    log.write(false, "into the void");
    log.write(true, "and again");
}

#[test]
fn the_previous_file_sits_beside_the_current_one() {
    // `uniwidged.log.1`, in the same directory: what `uniwidged log` reads
    // when the current file is shorter than it was asked for.
    let scratch = Scratch::new("beside");
    let previous = rotated(&scratch.log());
    assert_eq!(previous.parent(), Some(scratch.0.as_path()));
    assert_eq!(
        previous.file_name().and_then(|n| n.to_str()),
        Some("uniwidged.log.1")
    );
}
