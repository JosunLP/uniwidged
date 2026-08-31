//! What `uniwidged status` reads, and what it must refuse to believe.
//!
//! The file is written by one process and read by another, at a moment chosen
//! by a person who has just noticed something is wrong — so the two properties
//! worth a test are that a reader never sees half a document, and that a file
//! left behind by a desktop that is gone is not reported as the current one.

use super::*;

/// A directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "uniwidged-state-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Scratch(dir)
    }

    fn state(&self) -> PathBuf {
        self.0.join("uniwidged.state")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn a_desktop(pid: i32) -> State {
    State {
        pid,
        since: 1_756_400_000,
        shell: "X11 (_NET_WM_STATE_BELOW, 32-bit visual)".into(),
        widgets: vec![
            WidgetState {
                name: "clock".into(),
                version: "1.0.0".into(),
                x: 40,
                y: 40,
                width: 240,
                height: 160,
                output: Some("DP-1".into()),
                locked: true,
                health: Health::Running,
            },
            WidgetState {
                name: "broken".into(),
                version: String::new(),
                x: 300,
                y: 40,
                width: 200,
                height: 120,
                output: None,
                locked: false,
                health: Health::GivenUp,
            },
        ],
    }
}

#[test]
fn what_is_written_is_what_is_read() {
    let scratch = Scratch::new("roundtrip");
    let desktop = a_desktop(4242);
    desktop.write_to(&scratch.state());
    let read = State::read_for(&scratch.state(), 4242).expect("the state");
    assert_eq!(read, desktop);
}

#[test]
fn the_file_is_readable_by_a_person() {
    // Not a wire format: TOML with one `[[widget]]` per widget, so that
    // somebody who does not have this binary to hand — or whose binary is the
    // one that is misbehaving — can still read what the desktop said.
    let scratch = Scratch::new("readable");
    a_desktop(7).write_to(&scratch.state());
    let text = std::fs::read_to_string(scratch.state()).expect("the file");
    assert!(text.contains("pid = 7"), "{text}");
    assert!(text.contains("[[widget]]"), "{text}");
    assert!(text.contains("health = \"given-up\""), "{text}");
    assert!(text.contains("output = \"DP-1\""), "{text}");
}

#[test]
fn a_write_leaves_no_temporary_file_behind() {
    // Written to a temporary file and renamed over the old one, so a reader
    // never sees half a document. The temporary must be gone afterwards, or a
    // desktop that ran for a year would leave a year of them.
    let scratch = Scratch::new("atomic");
    a_desktop(1).write_to(&scratch.state());
    a_desktop(1).write_to(&scratch.state());
    let files: Vec<_> = std::fs::read_dir(&scratch.0)
        .expect("the directory")
        .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
        .collect();
    assert_eq!(files, vec!["uniwidged.state".to_string()], "{files:?}");
}

#[test]
fn a_file_left_by_a_desktop_that_is_gone_is_refused() {
    // The `SIGKILL` case. A new desktop is running under a new pid and has not
    // written yet; the old file is still there and describes widgets that are
    // not on the screen. Reporting those would be worse than reporting
    // nothing.
    let scratch = Scratch::new("stale");
    a_desktop(1000).write_to(&scratch.state());
    let why = State::read_for(&scratch.state(), 2000).expect_err("a stale file");
    assert!(why.contains("1000") && why.contains("2000"), "{why}");
    assert!(why.contains("stale"), "{why}");
}

#[test]
fn a_running_desktop_with_no_file_is_a_different_message_from_no_desktop() {
    // `current()` answers `Ok(None)` for no desktop, which is the common case
    // and not worth a word. A desktop that *is* running and has written
    // nothing is a fact worth reporting, because `status` has a question with
    // an answer and cannot give it.
    let scratch = Scratch::new("missing");
    let why = State::read_for(&scratch.state(), 33).expect_err("no file");
    assert!(why.contains("33") && why.contains("no state file"), "{why}");
}

#[test]
fn a_file_that_is_not_toml_says_which_file() {
    let scratch = Scratch::new("garbage");
    std::fs::write(scratch.state(), "this is not a state file").expect("write");
    let why = State::read_for(&scratch.state(), 1).expect_err("garbage");
    assert!(why.contains("uniwidged.state"), "{why}");
}

#[test]
fn a_file_from_a_newer_desktop_with_fields_this_one_lacks_still_reads() {
    // `serde(default)` on everything that arrived after the first version,
    // and unknown keys ignored: a `status` binary older than the desktop it is
    // asking still gets an answer rather than a parse error.
    let scratch = Scratch::new("forward");
    std::fs::write(
        scratch.state(),
        "pid = 5\nsince = 0\nfuture-field = true\n\n[[widget]]\nname = \"clock\"\n\
         version = \"1\"\nx = 0\ny = 0\nwidth = 10\nheight = 10\nhealth = \"running\"\n\
         something-new = 1\n",
    )
    .expect("write");
    let state = State::read_for(&scratch.state(), 5).expect("the state");
    assert_eq!(state.widgets.len(), 1);
    assert_eq!(state.widgets[0].health, Health::Running);
    assert!(!state.widgets[0].locked);
    assert_eq!(state.shell, "");
}

#[test]
fn health_reads_the_way_status_prints_it() {
    // What a person sees in `uniwidged status` and `uniwidged list`. "given up
    // on" rather than "suspended": the second is the supervisor's word, and
    // the first is what it means.
    assert_eq!(Health::Running.to_string(), "running");
    assert_eq!(Health::Restarting.to_string(), "restarting");
    assert_eq!(Health::GivenUp.to_string(), "given up on");
}
