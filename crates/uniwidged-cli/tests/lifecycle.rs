//! Installing a widget you did not write.
//!
//! The sandbox is what makes running somebody else's code reasonable once it is
//! running. These are the questions before that — is this the thing its author
//! published, is its author somebody this machine trusts, and can I look before
//! I agree? — driven through the actual binary, because the answer a user gets
//! is the one the command line prints.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A configuration directory that removes itself, standing in for a user's.
struct Machine(PathBuf);

impl Machine {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "uniwidged-lifecycle-{what}-{}-{}",
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

    /// Runs the CLI as this machine's user would.
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_uniwidged"))
            .args(args)
            // The whole of this machine's state — configuration, trusted keys
            // and widget data — hangs off one variable, which is what makes a
            // test like this possible at all.
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
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The clock, built once and copied where each test wants it.
fn reference_widget(into: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../widgets/clock");
    let built = Command::new(env!("CARGO"))
        .current_dir(&source)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .status()
        .expect("cargo");
    assert!(built.success(), "the clock widget should build");

    let component = into.join("clock.wasm");
    std::fs::copy(
        source.join("target/wasm32-wasip2/release/clock_widget.wasm"),
        &component,
    )
    .expect("copy the component");
    std::fs::copy(source.join("widget.toml"), into.join("clock.toml")).expect("copy the manifest");
    component
}

#[test]
fn an_unsigned_widget_is_refused_and_the_message_says_both_ways_out() {
    // The default. A user who was sent a `.wasm` should be told what to ask for
    // rather than have it quietly installed — and a developer building their
    // own should not be stuck.
    let machine = Machine::new("unsigned");
    let component = reference_widget(&machine.0);

    let said = machine.fails(&["install", component.to_str().unwrap()]);
    assert!(said.contains("not signed"), "{said}");
    assert!(said.contains("--allow-unsigned"), "it should say: {said}");
    assert!(said.contains("uniwidged trust"), "and: {said}");
    assert!(!machine.config().exists(), "and nothing was installed");
}

#[test]
fn the_guides_first_install_works_straight_from_cargos_output() {
    // `cargo build` writes the component under `target/` and no manifest
    // beside it. The guide's next command is `uniwidged install` on exactly
    // that path, and it used to be refused for a file nobody had told the
    // author to copy.
    let machine = Machine::new("from-target");
    reference_widget(&machine.0);
    let built = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../widgets/clock/target/wasm32-wasip2/release/clock_widget.wasm");
    assert!(
        !built.with_extension("toml").exists(),
        "the premise: nothing is beside it"
    );

    let said = machine.ok(&["install", built.to_str().unwrap(), "--allow-unsigned"]);
    assert!(
        said.contains("manifest:"),
        "it should say which manifest it used: {said}"
    );
    assert!(said.contains("widget.toml"), "{said}");
    assert!(machine.ok(&["list"]).contains("clock"));

    let config = std::fs::read_to_string(machine.config()).unwrap();
    assert!(
        config.contains("manifest = "),
        "and remember it, since the desktop would not look there: {config}"
    );
}

#[test]
fn a_relative_manifest_is_recorded_as_where_it_actually_is() {
    // The desktop reads the configuration from wherever it was started, so a
    // path recorded as typed would name a different file — or none — there.
    let machine = Machine::new("relative-manifest");
    let component = reference_widget(&machine.0);
    let elsewhere = machine.0.join("manifests");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::rename(machine.0.join("clock.toml"), elsewhere.join("clock.toml")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_uniwidged"))
        .current_dir(&machine.0)
        .args(["install", component.to_str().unwrap(), "--allow-unsigned"])
        .args(["--manifest", "manifests/clock.toml"])
        .env("UNIWIDGED_CONFIG", machine.config())
        .output()
        .expect("the CLI should be runnable");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let config = std::fs::read_to_string(machine.config()).unwrap();
    let recorded = elsewhere.join("clock.toml").canonicalize().unwrap();
    assert!(
        config.contains(&format!("manifest = \"{}\"", recorded.display())),
        "it should record the absolute path, {}: {config}",
        recorded.display()
    );
}

#[test]
fn a_developer_can_install_their_own_build_by_saying_so() {
    let machine = Machine::new("allow-unsigned");
    let component = reference_widget(&machine.0);

    let said = machine.ok(&["install", component.to_str().unwrap(), "--allow-unsigned"]);
    assert!(
        said.contains("NOT SIGNED"),
        "it should say so loudly rather than in passing: {said}"
    );
    assert!(machine.ok(&["list"]).contains("clock"));
}

#[test]
fn an_author_signs_a_user_trusts_and_the_widget_installs() {
    // The whole path, through the binary: keygen, sign, trust, install.
    let machine = Machine::new("signed");
    let component = reference_widget(&machine.0);
    let key = machine.0.join("signing-key");

    let generated = machine.ok(&["keygen", "--out", key.to_str().unwrap()]);
    let public = generated
        .lines()
        .find_map(|line| line.trim().strip_prefix("public key: "))
        .expect("keygen prints the public key")
        .to_string();
    assert_eq!(public.len(), 64);

    machine.ok(&[
        "sign",
        component.to_str().unwrap(),
        "--key",
        key.to_str().unwrap(),
    ]);
    assert!(
        machine.0.join("clock.wasm.sig").is_file(),
        "the signature goes beside the widget"
    );

    // Still refused: the signature is fine and nobody said the key was.
    let said = machine.fails(&["install", component.to_str().unwrap()]);
    assert!(said.contains("does not trust"), "{said}");
    assert!(said.contains("uniwidged trust"), "{said}");

    machine.ok(&["trust", &public, "--name", "jane"]);
    let installed = machine.ok(&["install", component.to_str().unwrap()]);
    assert!(installed.contains("signed by jane"), "{installed}");
}

#[test]
fn signing_a_fresh_build_puts_the_manifest_it_covered_beside_it() {
    // `sign` finds the crate's `widget.toml`, as `install` does. A signature
    // only verifies against the manifest bytes it covered, so the published
    // set has to carry those bytes as `<component>.toml` — otherwise an author
    // ships the .wasm and the .sig and nobody can install either.
    let machine = Machine::new("sign-from-target");
    let built = reference_widget(&machine.0);
    let krate = machine.0.join("my-widget");
    let release = krate.join("target/wasm32-wasip2/release");
    std::fs::create_dir_all(&release).unwrap();
    std::fs::write(krate.join("Cargo.toml"), "").unwrap();
    std::fs::rename(machine.0.join("clock.toml"), krate.join("widget.toml")).unwrap();
    let component = release.join("clock.wasm");
    std::fs::rename(&built, &component).unwrap();

    let key = machine.0.join("signing-key");
    let public = machine
        .ok(&["keygen", "--out", key.to_str().unwrap()])
        .lines()
        .find_map(|line| line.trim().strip_prefix("public key: "))
        .expect("a public key")
        .to_string();
    let said = machine.ok(&[
        "sign",
        component.to_str().unwrap(),
        "--key",
        key.to_str().unwrap(),
    ]);
    assert!(said.contains("publish all three"), "{said}");
    assert_eq!(
        std::fs::read(release.join("clock.toml")).unwrap(),
        std::fs::read(krate.join("widget.toml")).unwrap(),
        "the exact manifest the signature covers, beside the component"
    );

    // And the three files, moved anywhere together, install as signed.
    let published = machine.0.join("published");
    std::fs::create_dir_all(&published).unwrap();
    for name in ["clock.wasm", "clock.toml", "clock.wasm.sig"] {
        std::fs::copy(release.join(name), published.join(name)).unwrap();
    }
    machine.ok(&["trust", &public, "--name", "jane"]);
    let installed = machine.ok(&["install", published.join("clock.wasm").to_str().unwrap()]);
    assert!(installed.contains("signed by jane"), "{installed}");
}

#[cfg(unix)]
#[test]
fn signing_through_a_link_into_a_build_keeps_the_manifest_beside_the_link() {
    sign_through_a_link(false);
}

#[cfg(unix)]
#[test]
fn signing_through_a_link_inside_a_build_keeps_the_manifest_beside_the_link() {
    sign_through_a_link(true);
}

/// A link kept with its own manifest, pointing at a crate's build — from
/// outside `target/`, or from inside it: `sign` signs that manifest and
/// leaves it alone, rather than overwriting it with the crate's `widget.toml`.
#[cfg(unix)]
fn sign_through_a_link(inside_target: bool) {
    let machine = Machine::new(if inside_target {
        "sign-link-inside"
    } else {
        "sign-through-link"
    });
    let built = reference_widget(&machine.0);
    let krate = machine.0.join("my-widget");
    let release = krate.join("target/wasm32-wasip2/release");
    std::fs::create_dir_all(&release).unwrap();
    std::fs::write(krate.join("Cargo.toml"), "").unwrap();
    let original = std::fs::read_to_string(machine.0.join("clock.toml")).unwrap();
    std::fs::write(krate.join("widget.toml"), &original).unwrap();
    std::fs::rename(&built, release.join("clock.wasm")).unwrap();

    let linked = if inside_target {
        krate.join("target/linked")
    } else {
        machine.0.join("linked")
    };
    std::fs::create_dir_all(&linked).unwrap();
    std::os::unix::fs::symlink(release.join("clock.wasm"), linked.join("clock.wasm")).unwrap();
    let own = original.replace(
        "The time and the date, in your locale",
        "The clock, as published beside the link",
    );
    assert_ne!(own, original, "two different manifests");
    std::fs::write(linked.join("clock.toml"), &own).unwrap();

    let key = machine.0.join("signing-key");
    let public = machine
        .ok(&["keygen", "--out", key.to_str().unwrap()])
        .lines()
        .find_map(|line| line.trim().strip_prefix("public key: "))
        .expect("a public key")
        .to_string();
    machine.ok(&[
        "sign",
        linked.join("clock.wasm").to_str().unwrap(),
        "--key",
        key.to_str().unwrap(),
    ]);
    assert_eq!(
        std::fs::read_to_string(linked.join("clock.toml")).unwrap(),
        own,
        "the manifest beside the link is untouched"
    );
    assert_eq!(
        std::fs::read_to_string(krate.join("widget.toml")).unwrap(),
        original,
        "and so is the crate's"
    );

    // The signature covers the link's manifest, so the link installs as signed.
    machine.ok(&["trust", &public, "--name", "jane"]);
    let installed = machine.ok(&["install", linked.join("clock.wasm").to_str().unwrap()]);
    assert!(installed.contains("signed by jane"), "{installed}");
}

#[test]
fn changing_the_manifest_after_signing_is_caught() {
    // **The reason a signature covers the manifest too.** Without it, this
    // widens what the widget may do while keeping a signature the user trusts —
    // and the consent screen they read would be one the author never wrote.
    let machine = Machine::new("tampered");
    let component = reference_widget(&machine.0);
    let key = machine.0.join("signing-key");
    let public = machine
        .ok(&["keygen", "--out", key.to_str().unwrap()])
        .lines()
        .find_map(|line| line.trim().strip_prefix("public key: "))
        .expect("a public key")
        .to_string();
    machine.ok(&[
        "sign",
        component.to_str().unwrap(),
        "--key",
        key.to_str().unwrap(),
    ]);
    machine.ok(&["trust", &public, "--name", "jane"]);
    assert!(
        machine
            .run(&["install", component.to_str().unwrap()])
            .status
            .success()
    );

    // Now widen it, exactly as somebody passing the widget on would.
    let manifest = machine.0.join("clock.toml");
    let widened = std::fs::read_to_string(&manifest).unwrap().replace(
        "capabilities = [\"timer\", \"locale\"]",
        "capabilities = [\"timer\", \"locale\", \"net:*.evil.test\"]",
    );
    assert!(widened.contains("evil.test"), "the test's own edit landed");
    std::fs::write(&manifest, widened).unwrap();

    let said = machine.fails(&["install", component.to_str().unwrap()]);
    assert!(said.contains("does not check out"), "{said}");
    assert!(
        said.contains("manifest"),
        "it should name what changed: {said}"
    );
}

#[test]
fn a_signature_that_does_not_verify_cannot_be_waved_through() {
    // `--allow-unsigned` is for the absence of a claim. A signature that fails
    // is a positive claim that something is wrong, and no flag should get past
    // it.
    let machine = Machine::new("no-override");
    let component = reference_widget(&machine.0);
    std::fs::write(
        machine.0.join("clock.wasm.sig"),
        format!("uniwidged-sig-v1 {} {}\n", "ab".repeat(64), "cd".repeat(32)),
    )
    .unwrap();

    let said = machine.fails(&["install", component.to_str().unwrap(), "--allow-unsigned"]);
    assert!(said.contains("does not check out"), "{said}");
}

#[test]
fn withdrawing_trust_stops_the_next_install() {
    let machine = Machine::new("untrust");
    let component = reference_widget(&machine.0);
    let key = machine.0.join("signing-key");
    let public = machine
        .ok(&["keygen", "--out", key.to_str().unwrap()])
        .lines()
        .find_map(|line| line.trim().strip_prefix("public key: "))
        .expect("a public key")
        .to_string();
    machine.ok(&[
        "sign",
        component.to_str().unwrap(),
        "--key",
        key.to_str().unwrap(),
    ]);
    machine.ok(&["trust", &public]);
    assert!(machine.ok(&["trusted"]).contains(&public));
    assert!(
        machine
            .run(&["install", component.to_str().unwrap()])
            .status
            .success()
    );

    machine.ok(&["untrust", &public]);
    assert!(!machine.ok(&["trusted"]).contains(&public));
    let said = machine.fails(&["install", component.to_str().unwrap()]);
    assert!(said.contains("does not trust"), "{said}");
}

#[test]
fn check_reports_what_a_widget_asks_for_without_installing_it() {
    let machine = Machine::new("check");
    let component = reference_widget(&machine.0);

    let said = machine.ok(&["check", component.to_str().unwrap()]);
    assert!(said.contains("clock"), "{said}");
    assert!(said.contains("signature: none"), "{said}");
    assert!(
        said.contains("format dates and times"),
        "it should read as sentences: {said}"
    );
    assert!(
        said.contains("It starts, and draws"),
        "it should actually run it: {said}"
    );
    assert!(
        !machine.config().exists(),
        "and installing is a different verb"
    );
}

#[test]
fn check_refuses_something_that_is_not_a_widget() {
    let machine = Machine::new("not-a-widget");
    std::fs::write(machine.0.join("junk.wasm"), b"this is not a component").unwrap();
    std::fs::write(machine.0.join("junk.toml"), "name = \"junk\"\n").unwrap();

    let said = machine.fails(&["check", machine.0.join("junk.wasm").to_str().unwrap()]);
    assert!(said.contains("not a widget this host can load"), "{said}");
}

#[test]
fn a_widget_can_be_turned_off_without_losing_where_it_was_put() {
    // The difference between `disable` and `remove`, and the reason both exist:
    // turning a widget off to see whether it was the one making a noise should
    // not cost the half-hour somebody spent placing it.
    let machine = Machine::new("disable");
    let component = reference_widget(&machine.0);
    machine.ok(&["install", component.to_str().unwrap(), "--allow-unsigned"]);
    machine.ok(&["move", "clock", "--x", "900", "--y", "40"]);

    machine.ok(&["disable", "clock"]);
    let listed = machine.ok(&["list"]);
    assert!(listed.contains("(disabled)"), "{listed}");
    assert!(
        listed.contains("900,40"),
        "its placement is untouched: {listed}"
    );

    assert!(machine.ok(&["disable", "clock"]).contains("already"));
    machine.ok(&["enable", "clock"]);
    let listed = machine.ok(&["list"]);
    assert!(!listed.contains("(disabled)"), "{listed}");
    assert!(listed.contains("900,40"));
}

#[test]
fn removing_a_widget_keeps_its_data_unless_told_otherwise() {
    // Removing a widget is something people do to try one out and change their
    // minds. A note-taking widget that silently took the notes with it would be
    // a bug nobody could undo.
    let machine = Machine::new("remove");
    let component = reference_widget(&machine.0);
    machine.ok(&["install", component.to_str().unwrap(), "--allow-unsigned"]);

    // Stand in for what the widget would have written.
    let data = machine.0.join("widgets/clock");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("state"), b"remember me").unwrap();

    let said = machine.ok(&["remove", "clock"]);
    assert!(said.contains("removed"), "{said}");
    assert!(
        said.contains("--purge"),
        "it should say the data is there: {said}"
    );
    assert!(data.join("state").exists(), "and it really is");
    assert!(machine.ok(&["list"]).contains("nothing installed"));

    // And `--purge` does what it says.
    machine.ok(&["install", component.to_str().unwrap(), "--allow-unsigned"]);
    machine.ok(&["remove", "clock", "--purge"]);
    assert!(!data.exists(), "purge deletes the widget's directory");
}

#[test]
fn removing_something_that_is_not_installed_says_so() {
    let machine = Machine::new("missing");
    assert!(
        machine
            .fails(&["remove", "nothing"])
            .contains("not installed")
    );
    assert!(
        machine
            .fails(&["enable", "nothing"])
            .contains("not installed")
    );
}

#[test]
fn a_signing_key_is_not_left_where_anybody_can_read_it() {
    let machine = Machine::new("permissions");
    let key = machine.0.join("signing-key");
    machine.ok(&["keygen", "--out", key.to_str().unwrap()]);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&key).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o077,
            0,
            "a signing key must not be readable by anybody else, got {mode:o}"
        );
    }

    // And it will not be overwritten by a second run.
    assert!(
        machine
            .fails(&["keygen", "--out", key.to_str().unwrap()])
            .contains("already exists")
    );
}
