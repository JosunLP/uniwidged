//! `status`, `log` and `list --names` against a machine with no desktop on it.
//!
//! The common case for all three, and the one every other test in this crate
//! is in: a configuration on disk and nothing running against it. What is
//! asserted is that each says so in a sentence rather than in an error — no
//! desktop is not a failure — and that the one output meant for a script
//! (`--names`) is exactly the format a script needs. The case with a desktop
//! is `live.rs`, which needs a screen.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A configuration directory that removes itself, standing in for a user's.
struct Machine(PathBuf);

impl Machine {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "uniwidged-status-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Self(path)
    }

    fn config(&self) -> PathBuf {
        self.0.join("config.toml")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_uniwidged"))
            .args(args)
            .env("UNIWIDGED_CONFIG", self.config())
            .output()
            .expect("the CLI should be runnable")
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "`uniwidged {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn status_with_no_desktop_is_a_sentence_and_not_an_error() {
    let machine = Machine::new("status");
    let said = machine.ok(&["status"]);
    assert!(said.contains("no desktop is running"), "{said}");
    assert!(said.contains("uniwidged run"), "{said}");
}

#[test]
fn a_stale_pid_file_does_not_make_status_report_a_desktop() {
    // The `SIGKILL` case: a pid file and a state file left behind, naming a
    // process that is gone. Reporting its widgets as on the screen would be
    // worse than reporting nothing, so the pid is checked before the state
    // file is read at all.
    let machine = Machine::new("stale");
    // A pid that is certainly not a running uniwidged: this test's own
    // parent chain is `cargo`, and pid 2 is `kthreadd` on Linux — alive, and
    // never this program.
    std::fs::write(machine.0.join("uniwidged.pid"), "2\n").expect("pid file");
    std::fs::write(
        machine.0.join("uniwidged.state"),
        "pid = 2\nsince = 0\n\n[[widget]]\nname = \"ghost\"\nversion = \"1\"\n\
         x = 0\ny = 0\nwidth = 10\nheight = 10\nhealth = \"running\"\n",
    )
    .expect("state file");
    let said = machine.ok(&["status"]);
    assert!(said.contains("no desktop is running"), "{said}");
    assert!(!said.contains("ghost"), "{said}");
}

#[test]
fn log_with_nothing_logged_says_where_the_log_would_be() {
    let machine = Machine::new("log");
    let said = machine.ok(&["log"]);
    assert!(said.contains("nothing logged yet"), "{said}");
    assert!(said.contains("uniwidged.log"), "{said}");
}

#[test]
fn log_shows_the_last_lines_and_reads_the_rotated_file_when_short() {
    // The line somebody is looking for is most likely the one written just
    // before a rotation, so a request for more lines than the current file
    // holds reaches into the previous one.
    let machine = Machine::new("tail");
    std::fs::write(
        machine.0.join("uniwidged.log.1"),
        "2026-08-31 09:00:00   old one\n2026-08-31 09:00:01 ! old trouble\n",
    )
    .expect("previous");
    std::fs::write(
        machine.0.join("uniwidged.log"),
        "2026-08-31 10:00:00   new one\n2026-08-31 10:00:01   new two\n",
    )
    .expect("current");

    let said = machine.ok(&["log", "-n", "3"]);
    assert!(said.contains("old trouble") && said.contains("new one") && said.contains("new two"));
    assert!(!said.contains("old one"), "{said}");

    let said = machine.ok(&["log", "-n", "1"]);
    assert!(
        said.contains("new two") && !said.contains("new one"),
        "{said}"
    );

    let output = machine.run(&["log", "-n", "many"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("number"));
}

#[test]
fn list_names_is_one_name_per_line_and_nothing_else() {
    // What a shell completion reads. A sentence here would complete to a
    // sentence, and "nothing installed" is a name nobody has.
    let machine = Machine::new("names");
    assert_eq!(machine.ok(&["list", "--names"]), "");

    std::fs::write(
        machine.config(),
        "[[widget]]\nname = \"clock\"\ncomponent = \"/nowhere/clock.wasm\"\n\n\
         [[widget]]\nname = \"system-load\"\ncomponent = \"/nowhere/load.wasm\"\nenabled = false\n",
    )
    .expect("config");
    assert_eq!(machine.ok(&["list", "--names"]), "clock\nsystem-load\n");
    // And the ordinary listing still says what is disabled, which `--names`
    // deliberately does not: a disabled widget is still one `enable` can name.
    let said = machine.ok(&["list"]);
    assert!(said.contains("system-load (disabled)"), "{said}");
}

#[test]
fn completions_come_out_of_the_binary_for_every_shell() {
    let machine = Machine::new("completions");
    for shell in ["bash", "zsh", "fish"] {
        let script = machine.ok(&["completions", shell]);
        assert!(script.contains("uniwidged"), "{shell}: {script}");
        assert!(
            script.contains("status") && script.contains("configure"),
            "{shell}"
        );
    }
    let output = machine.run(&["completions", "csh"]);
    assert!(!output.status.success());
}
