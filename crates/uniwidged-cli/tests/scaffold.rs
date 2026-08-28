//! Does the scaffold produce a working widget?
//!
//! `ROADMAP.md` M8 says the framework is done when *somebody who has never seen
//! the repository writes and installs a widget from the guide alone*. That
//! cannot be asserted, but the mechanical half of it can: run `uniwidged new`,
//! build what it wrote for `wasm32-wasip2`, hand the result to the real
//! runtime, and click the button.
//!
//! A scaffold nobody compiles is a scaffold that stops compiling. This is the
//! test that notices — and it notices for the whole chain at once, because a
//! change to the SDK, the manifest format, the ABI or the component world all
//! surface here as the generated project failing to build.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use uniwidged_core::NodeId;
use uniwidged_runtime::capability::Policy;
use uniwidged_runtime::instance::{Event, Runtime};
use uniwidged_runtime::manifest::Manifest;
use uniwidged_runtime::services::Services;
use uniwidged_view::{Content, Node, View};

/// A directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "uniwidged-scaffold-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Self(path)
    }

    fn path(&self) -> String {
        self.0.display().to_string()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn uniwidged(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_uniwidged"))
        .args(args)
        .output()
        .expect("the CLI should be runnable")
}

fn find(node: &Node, id: NodeId) -> Option<&Node> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, id))
}

fn text_of(view: &View, id: &str) -> String {
    match &find(&view.root, NodeId::named(id))
        .unwrap_or_else(|| panic!("no node named {id} in {view:?}"))
        .content
    {
        Content::Text(text) => text.clone(),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn a_scaffolded_widget_builds_runs_and_responds() {
    let scratch = Scratch::new("build");
    let output = uniwidged(&["new", "demo", "--dir", &scratch.path()]);
    assert!(
        output.status.success(),
        "uniwidged new failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dir = scratch.0.join("demo");
    for file in [
        "Cargo.toml",
        "widget.toml",
        "src/lib.rs",
        "README.md",
        ".gitignore",
    ] {
        assert!(dir.join(file).is_file(), "{file} should have been written");
    }
    // Run inside a checkout, so it should have found the SDK rather than
    // written a version requirement nobody can resolve yet.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("crates"),
        "it should say which SDK it wrote against: {stdout}"
    );

    // The generated tests run natively, which is the promise the SDK makes to
    // widget authors: your logic is testable with no WASM toolchain.
    let natively = Command::new(env!("CARGO"))
        .current_dir(&dir)
        .arg("test")
        .status()
        .expect("cargo");
    assert!(natively.success(), "the generated tests should pass");

    let built = Command::new(env!("CARGO"))
        .current_dir(&dir)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .status()
        .expect("cargo");
    assert!(
        built.success(),
        "the generated widget should build for wasm32-wasip2. That target is \
         required: rustup target add wasm32-wasip2"
    );

    // And now the part that matters: the real runtime, the real ABI check, and
    // a widget nobody in this repository wrote by hand.
    let wasm = std::fs::read(dir.join("target/wasm32-wasip2/release/demo.wasm"))
        .expect("the component should be where the instructions say it is");
    let manifest = Manifest::parse(&std::fs::read_to_string(dir.join("widget.toml")).unwrap())
        .expect("the generated manifest should parse");
    assert!(
        manifest.capabilities.is_empty(),
        "a scaffold should start by asking for nothing"
    );

    let runtime = Runtime::new().expect("runtime");
    let component = runtime
        .compile(&wasm)
        .expect("the component should compile");
    let mut instance = runtime
        .instantiate(
            &component,
            Arc::new(manifest),
            // Deny-all, because the scaffold asks for nothing. A generated
            // widget that needed a grant to render would be teaching the wrong
            // thing.
            Policy::deny_all(),
            Services::none(),
        )
        .expect("the generated widget should instantiate");

    let view = instance.view().expect("it should render");
    assert_eq!(text_of(&view, "clicks"), "0");

    assert!(
        instance
            .on_event(Event::Clicked(NodeId::named("button")))
            .expect("the event should be delivered"),
        "the generated widget says its view would differ"
    );
    assert_eq!(
        text_of(&instance.view().expect("it should render again"), "clicks"),
        "1",
        "clicking the generated button should count"
    );
}

#[test]
fn a_scaffold_can_be_asked_for_capabilities_and_they_reach_the_manifest() {
    let scratch = Scratch::new("capabilities");
    let output = uniwidged(&[
        "new",
        "weather",
        "--dir",
        &scratch.path(),
        "--capability",
        "timer",
        "--capability",
        "net:api.open-meteo.com",
    ]);
    assert!(output.status.success());

    let manifest =
        Manifest::parse(&std::fs::read_to_string(scratch.0.join("weather/widget.toml")).unwrap())
            .expect("parses");
    assert_eq!(
        manifest.consent_summary(),
        vec![
            "update itself on a schedule",
            "talk to api.open-meteo.com over HTTPS"
        ],
        "what it asks for should read as sentences, because that is what a \
         user is shown"
    );
}

#[test]
fn a_scaffold_refuses_a_name_the_host_would_refuse() {
    // Better here than at install time: the name becomes a directory and a
    // keychain namespace, and by then there is a project built around it.
    let scratch = Scratch::new("bad-name");
    let output = uniwidged(&["new", "My Widget", "--dir", &scratch.path()]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not a usable widget name"),
        "it should say why: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!scratch.0.join("My Widget").exists(), "and write nothing");
}

#[test]
fn a_scaffold_refuses_an_unknown_capability_before_writing_anything() {
    let scratch = Scratch::new("bad-capability");
    let output = uniwidged(&[
        "new",
        "demo",
        "--dir",
        &scratch.path(),
        "--capability",
        "everything",
    ]);
    assert!(!output.status.success());
    assert!(!scratch.0.join("demo").exists());
}

#[test]
fn a_scaffold_will_not_overwrite_an_existing_project() {
    let scratch = Scratch::new("existing");
    assert!(
        uniwidged(&["new", "demo", "--dir", &scratch.path()])
            .status
            .success()
    );
    std::fs::write(scratch.0.join("demo/src/lib.rs"), "// mine").unwrap();

    let again = uniwidged(&["new", "demo", "--dir", &scratch.path()]);
    assert!(!again.status.success(), "a second run must refuse");
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("demo/src/lib.rs")).unwrap(),
        "// mine",
        "and must not have touched the work that was there"
    );
}
