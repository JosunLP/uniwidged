//! What a pid file is allowed to make happen.
//!
//! `SIGHUP`'s default action is **terminate**. A pid file left behind by a
//! crash names a number the kernel will reuse, so almost everything here is
//! about the cases where nothing must be sent — because the cost of getting
//! one of them wrong is killing a stranger's editor.

use super::*;

/// A directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> Self {
        use std::sync::atomic::AtomicU32;
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "uniwidged-daemon-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Self(path)
    }

    fn pid_file(&self) -> PathBuf {
        self.0.join("uniwidged.pid")
    }

    fn write_pid(&self, text: &str) -> PathBuf {
        let path = self.pid_file();
        std::fs::write(&path, text).expect("write");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn no_pid_file_is_not_an_error_worth_a_message() {
    // Every command that writes the configuration asks this, and most of the
    // time no desktop is running. A message each time would be noise, and
    // noise is how a real message gets missed.
    let scratch = Scratch::new("absent");
    assert_eq!(
        running_desktop(&scratch.pid_file()),
        Err(NotRunning::NoPidFile)
    );
}

#[test]
fn a_pid_file_that_does_not_hold_a_pid_says_so() {
    let scratch = Scratch::new("garbage");
    let path = scratch.write_pid("this is not a pid\n");
    assert!(matches!(
        running_desktop(&path),
        Err(NotRunning::Unreadable(_))
    ));
}

#[test]
fn a_pid_that_could_never_be_a_desktop_is_refused_before_anything_is_checked() {
    // `1` is init and `0` means "every process in my group", which `kill`
    // would happily accept. Neither is ever this program, and the cost of
    // being wrong is a signal whose default action is to terminate.
    let scratch = Scratch::new("impossible");
    for pid in ["0", "1", "-1"] {
        let path = scratch.write_pid(pid);
        assert!(
            matches!(running_desktop(&path), Err(NotRunning::Stale(_))),
            "pid {pid} should never be signalled"
        );
    }
}

#[test]
fn a_pid_belonging_to_a_process_that_is_not_uniwidged_is_stale() {
    // The case the name check exists for: a desktop crashed, its pid file
    // stayed, and the number now belongs to somebody's editor. This process is
    // alive and is `uniwidged-cli`'s test binary rather than `uniwidged`, so it
    // stands in for that editor exactly.
    let scratch = Scratch::new("somebody-else");
    let path = scratch.write_pid(&std::process::id().to_string());
    let answer = running_desktop(&path);
    // Linux reads `/proc/<pid>/comm`, macOS asks `proc_name`: both know the
    // name, and both must refuse.
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        assert_eq!(
            answer,
            Err(NotRunning::Stale(std::process::id() as i32)),
            "a live process that is not uniwidged must not be signalled"
        );
    } else {
        // Without either the check is liveness alone, which is weaker and is
        // documented as such rather than pretended to be the same.
        assert!(answer.is_ok());
    }
}

#[test]
fn a_pid_nothing_is_running_under_is_stale() {
    let scratch = Scratch::new("dead");
    // Well above any plausible live pid on a test machine, and checked rather
    // than assumed: if something *is* running there, the test would be
    // asserting the wrong thing, so it says so instead of failing oddly.
    let pid = 4_000_000;
    // SAFETY: signal 0 sends nothing; it performs the existence check only.
    if unsafe { libc::kill(pid, 0) } == 0 {
        eprintln!("skipping: something is running as pid {pid}");
        return;
    }
    let path = scratch.write_pid(&pid.to_string());
    assert_eq!(running_desktop(&path), Err(NotRunning::Stale(pid)));
}

#[test]
fn nudging_a_stale_pid_file_sends_nothing_and_says_why() {
    // The whole point. `nudge` goes through the same check, so there is no
    // path that signals a pid `running_desktop` would have refused.
    let scratch = Scratch::new("nudge-stale");
    let path = scratch.write_pid(&std::process::id().to_string());
    // Linux reads `/proc/<pid>/comm`, macOS asks `proc_name`: both know the
    // name, and both must refuse.
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        assert!(matches!(nudge(&path), Err(NotRunning::Stale(_))));
    }
    let gone = scratch.write_pid("4000000");
    // SAFETY: signal 0 sends nothing.
    if unsafe { libc::kill(4_000_000, 0) } != 0 {
        assert!(matches!(nudge(&gone), Err(NotRunning::Stale(_))));
    }
}

#[test]
fn a_pid_file_is_removed_when_the_desktop_stops() {
    // A stale file is handled everywhere, and it is still worth not leaving
    // one: a user who looks in their configuration directory should see a pid
    // file when a desktop is running and not otherwise.
    let scratch = Scratch::new("guard");
    let path = scratch.pid_file();
    {
        let guard = PidFile::claim(path.clone()).expect("claim");
        assert_eq!(guard.path(), path);
        let text = std::fs::read_to_string(&path).expect("the pid file");
        assert_eq!(text.trim(), std::process::id().to_string());
    }
    assert!(!path.exists(), "the guard should have removed it");
}

#[test]
fn a_claim_creates_the_directory_it_needs() {
    // A first run has no configuration directory at all, and a desktop that
    // refused to start over a missing folder would be refusing over its own
    // bookkeeping.
    let scratch = Scratch::new("mkdir");
    let path = scratch.0.join("nested/deeper/uniwidged.pid");
    let guard = PidFile::claim(path.clone()).expect("claim");
    assert!(path.exists());
    drop(guard);
}

#[test]
fn a_pending_signal_is_taken_exactly_once() {
    // The loop asks every turn and acts when the answer is yes. Reading
    // without clearing would reload forever; clearing without reading would
    // lose a change. `swap` is what makes a signal arriving between the two
    // impossible to drop.
    PENDING.store(true, Ordering::Relaxed);
    assert!(take_pending());
    assert!(!take_pending());
}
