//! Starting with the session, driven through the actual binary.
//!
//! Autostart is a feature nobody tests by hand more than once, because the way
//! to check it is to log out. So it is checked here instead — against a
//! session directory of the test's own, with `XDG_CONFIG_HOME` pointed at it,
//! which is exactly what a real session reads.
//!
//! Most of these are about what the command *will not* do. Writing a file into
//! somebody's session directory is the one thing in this program that changes
//! a machine outside uniwidged's own configuration, so the interesting cases
//! are the ones where it stops.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A session directory that removes itself.
struct Session(PathBuf);

impl Session {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "uniwidged-autostart-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Self(path)
    }

    fn entry(&self) -> PathBuf {
        self.0.join("autostart/uniwidged.desktop")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_uniwidged"))
            .arg("autostart")
            .args(args)
            // The session's directory, not uniwidged's own: the two are
            // deliberately separate, and this is the variable a desktop
            // environment actually reads.
            .env("XDG_CONFIG_HOME", &self.0)
            .env("UNIWIDGED_CONFIG", self.0.join("uniwidged/config.toml"))
            .output()
            .expect("the CLI should be runnable")
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "`uniwidged autostart {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn fails(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            !output.status.success(),
            "`uniwidged autostart {}` should have failed",
            args.join(" ")
        );
        String::from_utf8_lossy(&output.stderr).to_string()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn enabling_writes_an_entry_a_session_would_actually_run() {
    let session = Session::new("enable");
    session.ok(&["enable"]);

    let text = std::fs::read_to_string(session.entry()).expect("the entry should exist");
    assert!(text.starts_with("[Desktop Entry]"), "{text}");
    assert!(text.contains("Type=Application"), "{text}");

    // The `Exec` line is the whole point of the file, and the failure it has
    // to avoid is the one that is invisible until login: a bare `uniwidged`
    // resolved against a session's `PATH` rather than a shell's.
    let exec = text
        .lines()
        .find_map(|l| l.strip_prefix("Exec="))
        .expect("an Exec line");
    let (program, arguments) = exec.split_once(' ').expect("a program and its arguments");
    assert!(
        std::path::Path::new(program).is_absolute(),
        "Exec should name an absolute path, got {exec}"
    );
    assert!(
        std::path::Path::new(program).exists(),
        "Exec should name something that is there, got {exec}"
    );
    assert_eq!(arguments, "run");
}

#[test]
fn enabling_twice_is_not_two_desktops() {
    // The design is one process. An entry written a second time has to replace
    // the first rather than sit beside it, and the fixed file name is what
    // guarantees that — asserted, because a future version tempted to add a
    // timestamp or a profile to the name would break it silently.
    let session = Session::new("twice");
    session.ok(&["enable"]);
    session.ok(&["enable"]);
    let entries: Vec<_> = std::fs::read_dir(session.0.join("autostart"))
        .expect("the directory")
        .filter_map(Result::ok)
        .map(|e| e.file_name())
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
}

#[test]
fn disabling_removes_what_enabling_wrote_and_says_the_widgets_are_kept() {
    let session = Session::new("disable");
    session.ok(&["enable"]);
    let said = session.ok(&["disable"]);
    assert!(!session.entry().exists());
    // People expect one of these to imply the other, and it does not: turning
    // off autostart is not uninstalling, and the difference is worth a line of
    // output rather than a surprise.
    assert!(said.contains("untouched"), "{said}");
}

#[test]
fn disabling_when_it_was_never_enabled_is_not_an_error() {
    // A dotfiles script that runs `disable` unconditionally is a normal thing
    // to write, and it must not fail on a machine where nothing was enabled.
    let session = Session::new("not-enabled");
    let said = session.ok(&["disable"]);
    assert!(said.contains("not set to start"), "{said}");
}

#[test]
fn an_entry_somebody_wrote_by_hand_is_left_exactly_alone() {
    // The one case where this command touches something that is not its own.
    // Somebody who hand-wrote an entry — with their own flags, their own
    // wrapper script, an `OnlyShowIn` — has said something, and both replacing
    // it and deleting it would be this program deciding it knows better.
    let session = Session::new("theirs");
    let theirs = "[Desktop Entry]\nType=Application\nName=uniwidged\n\
                  Exec=/usr/local/bin/my-wrapper uniwidged run\n";
    std::fs::create_dir_all(session.0.join("autostart")).expect("dir");
    std::fs::write(session.entry(), theirs).expect("write");

    let refused = session.fails(&["enable"]);
    assert!(refused.contains("not written by uniwidged"), "{refused}");
    assert_eq!(
        std::fs::read_to_string(session.entry()).expect("still there"),
        theirs,
        "enable must not have touched it"
    );

    let refused = session.fails(&["disable"]);
    assert!(refused.contains("left alone"), "{refused}");
    assert_eq!(
        std::fs::read_to_string(session.entry()).expect("still there"),
        theirs,
        "disable must not have deleted it"
    );

    // And `status` says what is there rather than claiming autostart is off,
    // which would be the answer that sends somebody looking in the wrong file.
    let said = session.ok(&["status"]);
    assert!(said.contains("not written by uniwidged"), "{said}");
}

#[test]
fn status_says_which_of_the_three_states_this_is() {
    let session = Session::new("status");
    let off = session.ok(&["status"]);
    assert!(off.contains("does not start"), "{off}");

    session.ok(&["enable"]);
    let on = session.ok(&["status"]);
    assert!(on.contains("starts with your session"), "{on}");
    assert!(on.contains(&session.entry().display().to_string()), "{on}");

    // The third state, and the reason `status` exists at all: the entry is
    // fine and points at a binary that is not there any more — a `cargo
    // clean`, an upgrade, a move. At login that is silence, which is the
    // hardest kind of thing to debug from the symptom.
    let text = std::fs::read_to_string(session.entry()).expect("entry");
    let broken = text.replace("Exec=", "Exec=/nowhere/at/all/uniwidged #");
    std::fs::write(session.entry(), broken).expect("write");
    let stale = session.ok(&["status"]);
    assert!(stale.contains("is not there any more"), "{stale}");
}

#[test]
fn no_arguments_means_status_and_a_wrong_one_says_so() {
    let session = Session::new("words");
    assert_eq!(session.ok(&[]), session.ok(&["status"]));
    let refused = session.fails(&["enabel"]);
    assert!(refused.contains("enable|disable|status"), "{refused}");
}
