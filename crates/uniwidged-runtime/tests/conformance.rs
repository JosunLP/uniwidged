//! What the sandbox refuses.
//!
//! A sandbox tested only for what it permits is not tested. `widgets/conformance`
//! is a real component that attempts every capability on every frame and
//! reports what happened as its view; this reads that view back and asserts.
//!
//! It **builds the widget itself** rather than looking for a prebuilt file. A
//! conformance suite that skips when an artefact is missing is one that
//! eventually always skips, and nobody notices until it matters.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use uniwidged_core::NodeId;
use uniwidged_runtime::capability::{Capability, Policy};
use uniwidged_runtime::instance::{Event, Instance, Runtime, RuntimeError};
use uniwidged_runtime::limits::Limits;
use uniwidged_runtime::manifest::Manifest;
use uniwidged_view::{Content, Node, View};

fn widget_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../widgets/conformance")
}

/// Builds the conformance widget and returns the component bytes.
fn component_bytes() -> Vec<u8> {
    let dir = widget_dir();
    let status = Command::new(env!("CARGO"))
        .current_dir(&dir)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .status()
        .expect("cargo should be runnable");
    assert!(
        status.success(),
        "could not build the conformance widget in {}. \
         The wasm32-wasip2 target is required: rustup target add wasm32-wasip2",
        dir.display()
    );
    let wasm = dir.join("target/wasm32-wasip2/release/conformance_widget.wasm");
    std::fs::read(&wasm).unwrap_or_else(|e| panic!("{}: {e}", wasm.display()))
}

fn manifest() -> Manifest {
    let text = std::fs::read_to_string(widget_dir().join("widget.toml")).expect("widget.toml");
    Manifest::parse(&text).expect("the conformance manifest should parse")
}

/// Instantiates the widget under `policy`.
fn start(policy: Policy, limits: Option<Limits>) -> (Runtime, Instance) {
    let runtime = Runtime::new().expect("runtime");
    let component = runtime.compile(&component_bytes()).expect("compile");
    let mut manifest = manifest();
    if let Some(limits) = limits {
        manifest.limits = limits;
    }
    let instance = runtime
        .instantiate(&component, Arc::new(manifest), policy)
        .expect("instantiate");
    (runtime, instance)
}

fn find(node: &Node, id: NodeId) -> Option<&Node> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, id))
}

fn text_of(node: &Node) -> String {
    match &node.content {
        Content::Text(text) => text.clone(),
        other => panic!("expected text, got {other:?}"),
    }
}

/// The right-hand text of the row named `name`.
fn outcome(view: &View, name: &str) -> String {
    let row = find(&view.root, NodeId::named(name))
        .unwrap_or_else(|| panic!("the widget should report a row named {name}"));
    text_of(row.children.last().expect("the row should have an outcome"))
}

fn label(view: &View, name: &str) -> String {
    text_of(find(&view.root, NodeId::named(name)).expect("node"))
}

/// Everything the widget attempts that needs a grant.
const ATTEMPTS: &[&str] = &["locale", "measure", "net", "fs-read", "secrets", "metrics"];

#[test]
fn a_real_widget_instantiates_and_renders() {
    // The other half of the milestone: a component built for `wasm32-wasip2`,
    // instantiated by the real runtime, producing a view the host accepts.
    let (_runtime, mut instance) = start(Policy::deny_all(), None);
    let view = instance.view().expect("the widget should render");
    assert_eq!(
        label(&view, "started"),
        "started",
        "`start` should have run"
    );
    assert!(!view.root.children.is_empty());
}

#[test]
fn every_capability_is_refused_under_deny_all() {
    // The assertion the sandbox exists for. If any of these ever reads
    // anything but `not-granted`, there is a hole.
    let (_runtime, mut instance) = start(Policy::deny_all(), None);
    let view = instance.view().expect("render");
    for attempt in ATTEMPTS {
        assert_eq!(
            outcome(&view, attempt),
            "not-granted",
            "{attempt} was not refused"
        );
    }
}

#[test]
fn a_malformed_request_is_refused_before_the_grant_is_even_considered() {
    // A `file://` URL is not a network request that happens to be ungranted;
    // it is not a network request. Reporting it as ungranted would send its
    // author to the manifest to fix something the manifest cannot fix.
    let (_runtime, mut instance) = start(Policy::deny_all(), None);
    let view = instance.view().expect("render");
    assert_eq!(outcome(&view, "net-malformed"), "malformed");
    assert_eq!(outcome(&view, "fs-escape"), "malformed");
}

#[test]
fn granting_a_capability_changes_the_refusal_but_not_the_escape() {
    // With the grant in place the answer becomes `unavailable` — the host
    // cannot serve it yet — which is a different fact from `not-granted`, and
    // a widget should be able to tell them apart. What must *not* change is
    // the path escape: a grant for `cache` is not a grant for `../../.ssh`.
    let policy = Policy::from_granted([
        Capability::Locale,
        Capability::Measure,
        Capability::Net("example.com".into()),
        Capability::Fs("cache".into()),
        Capability::Secrets,
        Capability::Metrics,
    ]);
    let (_runtime, mut instance) = start(policy, None);
    let view = instance.view().expect("render");
    for attempt in ATTEMPTS {
        assert_eq!(
            outcome(&view, attempt),
            "unavailable",
            "{attempt} should be granted but unserved"
        );
    }
    assert_eq!(
        outcome(&view, "fs-escape"),
        "malformed",
        "a grant for `cache` must not reach outside the widget's directory"
    );
}

#[test]
fn a_grant_for_one_host_is_not_a_grant_for_another() {
    // The suffix attack, end to end: the widget asks for `example.com`, and a
    // policy naming a host that merely contains it must refuse.
    let policy = Policy::from_granted([Capability::Net("example.com.evil.test".into())]);
    let (_runtime, mut instance) = start(policy, None);
    let view = instance.view().expect("render");
    assert_eq!(outcome(&view, "net"), "not-granted");
}

#[test]
fn logging_always_works_and_reaches_the_host() {
    // The positive half: the host really is there, and the one capability that
    // needs no grant is wired up. A widget that cannot say what went wrong is
    // one nobody can help.
    let (_runtime, mut instance) = start(Policy::deny_all(), None);
    instance.view().expect("render");
    let log = instance.take_log();
    assert!(
        log.iter().any(|line| line.contains("conformance ran")),
        "the widget's log should reach the host, got {log:?}"
    );
    assert!(
        log[0].starts_with("[conformance]"),
        "a log line should name its widget, got {:?}",
        log[0]
    );
    assert!(instance.take_log().is_empty(), "taking should drain");
}

#[test]
fn events_reach_the_widget() {
    let (_runtime, mut instance) = start(Policy::deny_all(), None);
    assert_eq!(
        label(&instance.view().expect("render"), "events"),
        "events: 0"
    );
    assert!(
        instance
            .on_event(Event::Clicked(NodeId::named("anything")))
            .expect("event"),
        "the widget said its view would differ"
    );
    assert_eq!(
        label(&instance.view().expect("render"), "events"),
        "events: 1"
    );
}

#[test]
fn a_widget_that_never_returns_is_interrupted_at_its_deadline() {
    // The invariant that makes running other people's code on a desktop
    // reasonable: **the desktop never blocks on guest code**. The widget below
    // enters an unbounded loop with no allocation and no host call in it, so
    // nothing but wall-clock time can stop it — which is exactly why the
    // deadline is epochs rather than fuel, since fuel counts instructions and
    // a guest blocked on a granted host call would burn none.
    let limits = Limits {
        call_deadline: Duration::from_millis(20),
        ..Limits::default()
    };
    let (_runtime, mut instance) = start(Policy::deny_all(), Some(limits));
    instance
        .on_event(Event::Clicked(NodeId::named("spin")))
        .expect("the event itself returns");

    let began = std::time::Instant::now();
    let error = instance.view().expect_err("the loop cannot complete");
    let took = began.elapsed();

    assert!(matches!(error, RuntimeError::Trap(_)), "{error:?}");
    assert!(
        took < Duration::from_millis(500),
        "the host waited {took:?} on a widget that never returns"
    );
    assert!(
        instance.overran_deadline(),
        "this ended at the deadline, not in a panic — the difference decides \
         whether a supervisor tries again"
    );
}

#[test]
fn a_trapped_instance_is_finished_rather_than_retried() {
    // Wasmtime refuses to re-enter a component instance that has trapped, in
    // as many words. That is also the *right* behaviour: the widget was
    // stopped mid-function, so calling further into it would run its code in a
    // state its author never anticipated.
    //
    // This was worth finding out. The first design counted overruns per call
    // and expected a widget to survive two before being suspended; the second
    // call returned "cannot enter component instance" in 95 microseconds, and
    // the model had to become what it is now: a trap ends the instance, and
    // recovery is a fresh one.
    let limits = Limits {
        call_deadline: Duration::from_millis(10),
        ..Limits::default()
    };
    let (_runtime, mut instance) = start(Policy::deny_all(), Some(limits));
    instance
        .on_event(Event::Clicked(NodeId::named("spin")))
        .expect("event");
    let _ = instance.view();

    assert!(instance.is_dead());
    for again in [
        instance.view().err(),
        instance.next_wake().err(),
        instance.on_event(Event::Configured).err(),
    ] {
        assert!(
            matches!(again, Some(RuntimeError::Dead)),
            "a dead instance must refuse plainly, got {again:?}"
        );
    }
}

#[test]
fn a_fresh_instance_recovers_a_widget_that_died() {
    // The actual recovery path, and the reason a trap being terminal is not a
    // problem: a widget's state is derived from its data, so a new instance is
    // a working widget again. What a supervisor decides is *how many times* —
    // see `Health` — not whether it is possible.
    let runtime = Runtime::new().expect("runtime");
    let component = runtime.compile(&component_bytes()).expect("compile");
    let manifest = Arc::new(Manifest {
        limits: Limits {
            call_deadline: Duration::from_millis(10),
            ..Limits::default()
        },
        ..manifest()
    });

    let mut first = runtime
        .instantiate(&component, manifest.clone(), Policy::deny_all())
        .expect("first instance");
    first
        .on_event(Event::Clicked(NodeId::named("spin")))
        .expect("event");
    let _ = first.view();
    assert!(first.is_dead());

    let mut second = runtime
        .instantiate(&component, manifest, Policy::deny_all())
        .expect("a second instance of the same component");
    let view = second.view().expect("the fresh instance renders");
    assert_eq!(label(&view, "started"), "started");
    assert_eq!(
        label(&view, "events"),
        "events: 0",
        "a fresh instance starts from nothing, which is what makes this safe"
    );
    assert!(!second.is_dead());
}

#[test]
fn a_widget_that_tries_to_take_the_machines_memory_is_stopped() {
    // The other half of containment, and the half that has nothing to do with
    // capabilities: a widget granted *nothing* can still allocate until the
    // machine swaps. The `ResourceLimiter` refuses the growth, the guest's
    // allocator gets nothing back, and Rust aborts — which reaches us as a
    // trap. A cap nobody has watched fail is a cap nobody knows works.
    let limits = Limits {
        memory_bytes: 16 * 1024 * 1024,
        // Generous, so that what stops the widget is unambiguously the memory
        // cap and not the clock.
        call_deadline: Duration::from_secs(5),
        ..Limits::default()
    };
    let (_runtime, mut instance) = start(Policy::deny_all(), Some(limits));
    instance
        .on_event(Event::Clicked(NodeId::named("grow")))
        .expect("event");

    let began = std::time::Instant::now();
    let error = instance.view().expect_err("the allocation cannot succeed");
    assert!(matches!(error, RuntimeError::Trap(_)), "{error:?}");
    assert!(
        !instance.overran_deadline(),
        "this should have been stopped by the memory cap, not by the clock"
    );
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "the cap should bite long before the deadline"
    );
}

#[test]
fn a_widget_that_asks_for_no_schedule_gets_none() {
    // The conformance widget has no timer capability and asks for nothing, and
    // the host has to see that as "never" rather than as a default interval.
    let (_runtime, mut instance) = start(Policy::deny_all(), None);
    assert_eq!(
        instance.next_wake().expect("wake"),
        uniwidged_core::Wake::Never
    );
}

#[test]
fn the_manifest_travels_with_the_instance() {
    // The name reaches the log prefix, the data directory and the keychain
    // namespace, so the instance has to keep it rather than the caller.
    let (_runtime, instance) = start(Policy::deny_all(), None);
    assert_eq!(instance.manifest().name, "conformance");
    assert!(instance.manifest().capabilities.is_empty());
}
