//! Telling a running desktop, driven through the actual binary.
//!
//! The dangerous half of this feature is not the signal, it is the pid: a
//! stale file names a number the kernel has reused, and `SIGHUP`'s default
//! action is to terminate. So these are mostly about the cases where nothing
//! is sent.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Machine(PathBuf);

impl Machine {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "uniwidged-reload-{what}-{}-{}",
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

    fn pid_file(&self) -> PathBuf {
        self.0.join("uniwidged.pid")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_uniwidged"))
            .args(args)
            .env("UNIWIDGED_CONFIG", self.config())
            .output()
            .expect("the CLI should be runnable")
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn reloading_with_no_desktop_running_says_so_rather_than_pretending() {
    let machine = Machine::new("none");
    let output = machine.run(&["reload"]);
    assert!(!output.status.success());
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(said.contains("no desktop is running"), "{said}");
}

#[test]
fn a_pid_file_naming_something_that_is_not_uniwidged_is_never_signalled() {
    // The case the name check exists for: a desktop crashed, its pid file
    // stayed, and the number now belongs to somebody's editor. This test's own
    // process stands in for that editor — it is alive, and it is not
    // `uniwidged`.
    //
    // If this check were wrong, running the suite would send `SIGHUP` to the
    // test harness, whose default action is to terminate. The test would not
    // fail; it would take the run with it.
    let machine = Machine::new("stranger");
    std::fs::write(machine.pid_file(), std::process::id().to_string()).expect("write");

    let output = machine.run(&["reload"]);
    // Every platform can tell now: Linux reads `/proc/<pid>/comm`, macOS asks
    // `proc_name`, and on Windows only a running desktop has the reload event
    // its pid names.
    assert!(!output.status.success());
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(said.contains("is gone"), "{said}");
    // And this process is still here to assert it.
    assert_eq!(std::process::id(), std::process::id());
}

#[test]
fn a_pid_file_holding_nonsense_is_refused_rather_than_parsed_loosely() {
    let machine = Machine::new("garbage");
    for nonsense in ["", "  ", "pid=17", "1", "0", "-4"] {
        std::fs::write(machine.pid_file(), nonsense).expect("write");
        let output = machine.run(&["reload"]);
        assert!(
            !output.status.success(),
            "`{nonsense}` should not be signalled"
        );
    }
}

#[test]
fn writing_the_configuration_with_no_desktop_running_is_quiet_and_succeeds() {
    // Every command that writes the configuration asks whether a desktop is
    // running, and most of the time none is — on a server, in a script, in
    // this suite. A message each time would be noise, and noise is how a real
    // message gets missed.
    let machine = Machine::new("quiet");
    let component = build_widget(&machine.0, "system-load", "system_load_widget.wasm");
    let installed = machine.run(&[
        "install",
        component.to_str().expect("path"),
        "--allow-unsigned",
    ]);
    assert!(installed.status.success());

    let output = machine.run(&["configure", "system-load", "--set", "warn-at=0.9"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(said.contains("warn-at = 0.9"), "{said}");
    assert!(
        !said.contains("has been told"),
        "there is no desktop to have told: {said}"
    );
}

fn build_widget(into: &std::path::Path, name: &str, artefact: &str) -> PathBuf {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../widgets")
        .join(name);
    let built = Command::new(env!("CARGO"))
        .current_dir(&source)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .status()
        .expect("cargo");
    assert!(built.success(), "the {name} widget should build");
    let component = into.join(format!("{name}.wasm"));
    std::fs::copy(
        source.join("target/wasm32-wasip2/release").join(artefact),
        &component,
    )
    .expect("copy the component");
    std::fs::copy(
        source.join("widget.toml"),
        into.join(format!("{name}.toml")),
    )
    .expect("copy the manifest");
    component
}
