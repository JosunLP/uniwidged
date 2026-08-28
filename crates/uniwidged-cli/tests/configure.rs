//! `uniwidged configure`, driven through the actual binary.
//!
//! The point of declaring settings rather than handing a widget a string map
//! is that a widget can be written with no branch for a value it cannot
//! handle. That is only true if the refusals happen *here* — at the moment
//! somebody types, where they can be told what went wrong and what the shape
//! is. So most of this is about what the command will not write.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A configuration directory that removes itself.
struct Machine(PathBuf);

impl Machine {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "uniwidged-configure-{what}-{}-{}",
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

    fn fails(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            !output.status.success(),
            "`uniwidged {}` should have failed, and said: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout)
        );
        String::from_utf8_lossy(&output.stderr).to_string()
    }

    /// Installs the reference widget that declares settings.
    fn install_system_load(&self) {
        let component = build_system_load(&self.0);
        self.ok(&[
            "install",
            component.to_str().expect("path"),
            "--allow-unsigned",
        ]);
    }

    fn config_text(&self) -> String {
        std::fs::read_to_string(self.config()).expect("the configuration")
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `system-load`, which is the reference widget for settings as well as for
/// `metrics` — built rather than looked for, like everything else here.
fn build_system_load(into: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../widgets/system-load");
    let built = Command::new(env!("CARGO"))
        .current_dir(&source)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .status()
        .expect("cargo");
    assert!(built.success(), "the system-load widget should build");

    let component = into.join("system-load.wasm");
    std::fs::copy(
        source.join("target/wasm32-wasip2/release/system_load_widget.wasm"),
        &component,
    )
    .expect("copy the component");
    std::fs::copy(source.join("widget.toml"), into.join("system-load.toml"))
        .expect("copy the manifest");
    component
}

#[test]
fn showing_says_what_the_widget_takes_and_what_it_is_set_to() {
    let machine = Machine::new("show");
    machine.install_system_load();

    let said = machine.ok(&["configure", "system-load"]);
    assert!(said.contains("warn-at"), "{said}");
    assert!(said.contains("Warn above"), "the label: {said}");
    // The shape, because a form somebody has to guess is a form guessed wrong.
    assert!(said.contains("a number from 0 to 1"), "{said}");
    // And which values are the widget's own rather than the user's, so that
    // "why is this 0.6?" has an answer without opening the manifest.
    assert!(said.contains("(default)"), "{said}");
}

#[test]
fn setting_a_value_writes_only_what_differs_from_the_default() {
    let machine = Machine::new("set");
    machine.install_system_load();
    // A fresh installation records nothing: writing the defaults out would pin
    // this version's defaults into the file, so a widget that later changed
    // its mind about a sensible starting point would never reach the people
    // who never touched it.
    assert!(!machine.config_text().contains("settings"));

    machine.ok(&["configure", "system-load", "--set", "warn-at=0.9"]);
    let text = machine.config_text();
    assert!(text.contains("[widget.settings]"), "{text}");
    assert!(text.contains("warn-at = 0.9"), "{text}");
    // The one that was not touched is still absent.
    assert!(!text.contains("critical-at"), "{text}");

    let said = machine.ok(&["configure", "system-load"]);
    assert!(said.contains("warn-at = 0.9"), "{said}");
}

#[test]
fn two_settings_change_in_one_command() {
    // A form is saved all at once, and a command line that cannot express that
    // makes somebody run it twice — and leaves the desktop half-configured in
    // between.
    let machine = Machine::new("two");
    machine.install_system_load();
    machine.ok(&[
        "configure",
        "system-load",
        "--set",
        "warn-at=0.5",
        "--set",
        "critical-at=0.75",
    ]);
    let text = machine.config_text();
    assert!(text.contains("warn-at = 0.5"), "{text}");
    assert!(text.contains("critical-at = 0.75"), "{text}");
}

#[test]
fn setting_one_leaves_the_others_where_they_were() {
    let machine = Machine::new("leave");
    machine.install_system_load();
    machine.ok(&["configure", "system-load", "--set", "warn-at=0.5"]);
    machine.ok(&["configure", "system-load", "--set", "critical-at=0.75"]);
    let text = machine.config_text();
    assert!(
        text.contains("warn-at = 0.5"),
        "the first should survive: {text}"
    );
    assert!(text.contains("critical-at = 0.75"), "{text}");
}

#[test]
fn a_value_the_widget_would_not_accept_never_reaches_the_file() {
    // The whole reason settings are declared. If any of these were written, a
    // widget would have to have a branch for it.
    let machine = Machine::new("refused");
    machine.install_system_load();

    for bad in ["warn-at=banana", "warn-at=2", "warn-at=-1", "warn-at=true"] {
        let said = machine.fails(&["configure", "system-load", "--set", bad]);
        assert!(!said.is_empty(), "`{bad}` should say why");
        assert!(
            !machine.config_text().contains("settings"),
            "`{bad}` should not have been written"
        );
    }
    // And the message says what would be acceptable, because the fix is
    // usually one character away.
    let said = machine.fails(&["configure", "system-load", "--set", "warn-at=2"]);
    assert!(said.contains("between 0 and 1"), "{said}");
}

#[test]
fn a_key_the_widget_does_not_declare_is_refused_and_the_known_ones_are_listed() {
    let machine = Machine::new("unknown-key");
    machine.install_system_load();
    let said = machine.fails(&["configure", "system-load", "--set", "warn-att=0.9"]);
    assert!(said.contains("does not declare"), "{said}");
    // A typo is the likeliest reason to be here.
    assert!(said.contains("warn-at"), "{said}");
    assert!(said.contains("critical-at"), "{said}");
}

#[test]
fn an_assignment_that_is_not_key_equals_value_says_so() {
    let machine = Machine::new("shape");
    machine.install_system_load();
    let said = machine.fails(&["configure", "system-load", "--set", "warn-at"]);
    assert!(said.contains("key=value"), "{said}");
}

#[test]
fn a_text_value_may_contain_an_equals_sign() {
    // `--set k=v` splits on the *first* `=`: a key cannot contain one and a
    // text value perfectly well can.
    let machine = Machine::new("equals");
    let component = build_conformance(&machine.0);
    machine.ok(&[
        "install",
        component.to_str().expect("path"),
        "--allow-unsigned",
    ]);
    machine.ok(&["configure", "conformance", "--set", "some-text=a=b"]);
    let text = machine.config_text();
    assert!(text.contains("some-text = \"a=b\""), "{text}");
}

#[test]
fn every_declared_kind_can_be_set_and_reset() {
    // One of each shape, through the whole path: a command line, the parser,
    // the file, and back out again.
    let machine = Machine::new("kinds");
    let component = build_conformance(&machine.0);
    machine.ok(&[
        "install",
        component.to_str().expect("path"),
        "--allow-unsigned",
    ]);
    machine.ok(&[
        "configure",
        "conformance",
        "--set",
        "some-text=hello",
        "--set",
        "some-number=42",
        "--set",
        "some-toggle=true",
        "--set",
        "some-choice=two",
    ]);
    let text = machine.config_text();
    assert!(text.contains("some-text = \"hello\""), "{text}");
    assert!(text.contains("some-number = 42"), "{text}");
    assert!(text.contains("some-toggle = true"), "{text}");
    assert!(text.contains("some-choice = \"two\""), "{text}");

    // A choice is exact: a near-miss is refused rather than matched loosely.
    let said = machine.fails(&["configure", "conformance", "--set", "some-choice=Two"]);
    assert!(said.contains("one, two"), "the options: {said}");
    // A toggle takes exactly two words, because `yes` reads as true to
    // somebody and as a typo to somebody else.
    assert!(
        machine
            .fails(&["configure", "conformance", "--set", "some-toggle=yes"])
            .contains("true"),
    );

    machine.ok(&["configure", "conformance", "--reset", "some-number"]);
    let text = machine.config_text();
    assert!(
        !text.contains("some-number"),
        "reset should remove it: {text}"
    );
    assert!(text.contains("some-text"), "and leave the rest: {text}");

    machine.ok(&["configure", "conformance", "--reset-all"]);
    assert!(
        !machine.config_text().contains("widget.settings"),
        "everything at its default is an empty section"
    );
}

#[test]
fn a_widget_with_no_settings_says_so_rather_than_showing_an_empty_list() {
    let machine = Machine::new("none");
    let component = build_clock(&machine.0);
    machine.ok(&[
        "install",
        component.to_str().expect("path"),
        "--allow-unsigned",
    ]);
    let said = machine.ok(&["configure", "clock"]);
    assert!(said.contains("declares no settings"), "{said}");
    let refused = machine.fails(&["configure", "clock", "--set", "anything=1"]);
    assert!(refused.contains("no settings"), "{refused}");
}

#[test]
fn a_widget_that_is_not_installed_says_which_command_lists_them() {
    let machine = Machine::new("missing");
    let said = machine.fails(&["configure", "nothing-here"]);
    assert!(said.contains("uniwidged list"), "{said}");
}

#[test]
fn a_stale_key_in_a_hand_edited_file_is_reported_rather_than_hidden() {
    // What an upgrade produces: a widget dropped a knob in its new version and
    // the user's file still names it. Refusing to start over that would make
    // every upgrade a support question; using the default and saying nothing
    // would make a typo invisible for a week.
    let machine = Machine::new("stale");
    machine.install_system_load();
    machine.ok(&["configure", "system-load", "--set", "warn-at=0.9"]);

    let text = machine.config_text().replace(
        "[widget.settings]",
        "[widget.settings]\nsomething-removed = 1",
    );
    std::fs::write(machine.config(), text).expect("write");

    let said = machine.ok(&["configure", "system-load"]);
    assert!(said.contains("something-removed"), "{said}");
    assert!(said.contains("ignored"), "{said}");
    // And the setting that is still declared still reads back.
    assert!(said.contains("warn-at = 0.9"), "{said}");
}

/// The conformance widget, which declares one setting of every kind.
fn build_conformance(into: &Path) -> PathBuf {
    build(into, "conformance", "conformance_widget.wasm")
}

fn build_clock(into: &Path) -> PathBuf {
    build(into, "clock", "clock_widget.wasm")
}

fn build(into: &Path, name: &str, artefact: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
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
