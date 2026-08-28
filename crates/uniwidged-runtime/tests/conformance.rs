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
use uniwidged_runtime::services::{Commands, CpuSampler, DataDir, Locale, Services, TextMetrics};
use uniwidged_runtime::tasks::{TaskHandle, Tasks};
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

/// Instantiates the widget under `policy`, with a host that can serve nothing.
///
/// The default on purpose: most of what this file asserts is about the gate,
/// and a gate is tested by what it refuses. The tests that are about what is
/// *behind* the gate say so by passing their own [`Services`].
fn start(policy: Policy, limits: Option<Limits>) -> (Runtime, Instance) {
    start_with(policy, limits, Services::none())
}

fn start_with(policy: Policy, limits: Option<Limits>, services: Services) -> (Runtime, Instance) {
    let runtime = Runtime::new().expect("runtime");
    let component = runtime.compile(&component_bytes()).expect("compile");
    let mut manifest = manifest();
    if let Some(limits) = limits {
        manifest.limits = limits;
    }
    let instance = runtime
        .instantiate(&component, Arc::new(manifest), policy, services)
        .expect("instantiate");
    (runtime, instance)
}

/// A directory that removes itself, so a test that writes leaves nothing.
struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "uniwidged-conformance-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }

    /// Where this host would put the conformance widget's own directory.
    fn widget_dir(&self) -> PathBuf {
        self.0.join("conformance")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Everything a host that can serve `fs`, `locale`, `measure` and `metrics`
/// hands one widget.
///
/// UTC, so the strings a test expects are the same on every machine it runs on.
fn served(scratch: &Scratch) -> Services {
    served_with_tasks(scratch, Arc::new(Tasks::default()))
}

/// The same, sharing a task registry the test can look into.
fn served_with_tasks(scratch: &Scratch, tasks: Arc<Tasks>) -> Services {
    Services::none()
        .with_data_dir(DataDir::new(scratch.widget_dir()))
        .with_locale(Locale::default())
        .with_text_metrics(Arc::new(Ruler))
        .with_metrics(Arc::new(CpuSampler::new()))
        .with_tasks(
            TaskHandle::new(uniwidged_core::WidgetId(1), tasks)
                .with_commands(Arc::new(Commands::new()))
                .with_http(Arc::new(uniwidged_runtime::net::Http::new())),
        )
}

/// A text measurer with no fonts in it.
///
/// The real one lives in `uniwidged-compose` and is tested there against the
/// layout it has to agree with. What this file is testing is the *boundary* —
/// that a granted `measure` reaches the host at all, with the role the widget
/// named — so it uses something whose answers are arithmetic rather than
/// typography, and can therefore be asserted exactly.
struct Ruler;

impl TextMetrics for Ruler {
    fn width(&self, text: &str, role: uniwidged_view::Role) -> f32 {
        let per_character = match role {
            uniwidged_view::Role::Display => 20.0,
            _ => 7.0,
        };
        text.chars().count() as f32 * per_character
    }
}

/// The grants the served tests use: everything the widget attempts.
fn everything() -> Policy {
    Policy::from_granted([
        Capability::Locale,
        Capability::Measure,
        Capability::Net("conformance.invalid".into()),
        Capability::Fs("cache".into()),
        Capability::Secrets,
        Capability::Metrics,
        Capability::Command("echo".into()),
    ])
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
const ATTEMPTS: &[&str] = &[
    "locale",
    "locale-date",
    "measure",
    "net",
    "fs-read",
    "fs-roundtrip",
    "secrets",
    "metrics",
    "command",
];

/// The ones this host still has nothing behind, however much is granted.
const UNSERVED: &[&str] = &["secrets"];

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
fn a_grant_a_host_cannot_serve_says_so_rather_than_refusing() {
    // Two different facts, and a widget can act on the difference: with the
    // grant in place but nothing behind it the answer becomes `unavailable`,
    // where `not-granted` would have sent its author to fix a manifest that is
    // already right. What must *not* change is the path escape: a grant for
    // `cache` is not a grant for `../../.ssh`.
    let (_runtime, mut instance) = start(everything(), None);
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
fn a_served_capability_returns_what_it_was_asked_for() {
    // The positive half of the gate, and the half a sandbox suite usually
    // omits: a capability that is granted *and* served has to actually work.
    // The values are asserted rather than merely the absence of an error,
    // because a host that answers `format_time` wrongly is not conformant
    // either.
    let scratch = Scratch::new("served");
    let (_runtime, mut instance) = start_with(everything(), None, served(&scratch));
    let view = instance.view().expect("render");

    assert_eq!(outcome(&view, "locale"), "00:00");
    assert_eq!(outcome(&view, "locale-date"), "1970-01-01");
    // Nothing has written this one, and a first run finding nothing is the
    // normal case rather than the failure case.
    assert_eq!(outcome(&view, "fs-read"), "absent");
    assert_eq!(
        outcome(&view, "fs-roundtrip"),
        "roundtrip",
        "what the widget wrote should be what it reads back"
    );
    // The widget checked the relationship itself — a longer string is wider,
    // an empty one is nothing — because the pixels depend on the host's fonts
    // and the relationship does not.
    assert_eq!(outcome(&view, "measure"), "measured");
    assert_eq!(outcome(&view, "metrics"), "allowed");
    // Asynchronous by design: the first view can only report that the request
    // was *started*. The next test watches it finish.
    assert_eq!(outcome(&view, "command"), "started");
    // Likewise `net` — a token came back, which is what "allowed" means for
    // something that has not happened yet. Where it goes and what it refuses
    // on the way is `net`'s own suite, against a server it starts itself.
    assert_eq!(outcome(&view, "net"), "allowed");

    // Everything still unbuilt says so, in the same view, so this test fails
    // when one of them is implemented and nobody updated the list.
    for attempt in UNSERVED {
        assert_eq!(outcome(&view, attempt), "unavailable", "{attempt}");
    }
    assert_eq!(outcome(&view, "fs-escape"), "malformed");
}

#[test]
fn what_a_widget_writes_lands_in_its_own_directory_and_stays_there() {
    // Durability is what makes a restart cheap: a widget's in-memory state is
    // derived from its data, so a fresh instance is a working widget again —
    // *if* the data outlived the instance. This checks both ends of that, and
    // checks the file is where the host said it would be rather than merely
    // somewhere.
    let scratch = Scratch::new("durable");
    let expected = scratch.widget_dir().join("cache/roundtrip.bin");

    {
        let (_runtime, mut instance) = start_with(everything(), None, served(&scratch));
        assert_eq!(
            outcome(&instance.view().expect("render"), "fs-roundtrip"),
            "roundtrip"
        );
    }
    assert_eq!(
        std::fs::read(&expected).expect("the widget's file should be on disk"),
        b"uniwidged-roundtrip-v1",
        "{} should hold what the widget wrote",
        expected.display()
    );

    // A second instance — which is what a restart is — finds it there. It is
    // read through `fs-read` rather than the round trip, because a round trip
    // would write it again and prove nothing.
    std::fs::copy(&expected, scratch.widget_dir().join("cache/today.json")).expect("copy");
    let (_runtime, mut instance) = start_with(everything(), None, served(&scratch));
    assert_eq!(
        outcome(&instance.view().expect("render"), "fs-read"),
        "present",
        "a fresh instance should find what the last one left"
    );
}

#[test]
fn a_host_with_storage_still_refuses_a_path_out_of_the_directory() {
    // The escape has to be refused by the *same* host that would have served a
    // legitimate path — a check that only holds when there is nothing behind
    // it is not a check.
    let scratch = Scratch::new("escape");
    let (_runtime, mut instance) = start_with(everything(), None, served(&scratch));
    let view = instance.view().expect("render");
    assert_eq!(outcome(&view, "fs-escape"), "malformed");
    assert!(
        !scratch.0.join(".ssh").exists(),
        "nothing should have been created outside the widget's directory"
    );
}

#[test]
fn a_widget_granted_storage_it_never_asked_for_still_cannot_use_it() {
    // The grant is per subpath, and the host having a directory does not widen
    // it. `fs:cache` is a grant for `cache`, and the round trip writes there —
    // so this policy, which grants a *different* subpath, must refuse the same
    // call the previous test allowed.
    let scratch = Scratch::new("subpath");
    let policy = Policy::from_granted([Capability::Fs("state".into())]);
    let (_runtime, mut instance) = start_with(policy, None, served(&scratch));
    let view = instance.view().expect("render");
    assert_eq!(outcome(&view, "fs-roundtrip"), "not-granted");
    assert!(
        !scratch.widget_dir().join("cache").exists(),
        "a refused write must not create the directory it was refused"
    );
}

#[test]
fn a_grant_for_one_host_is_not_a_grant_for_another() {
    // The suffix attack, end to end: the widget asks for `conformance.invalid`,
    // and a policy naming a host that merely contains it must refuse.
    let policy = Policy::from_granted([Capability::Net("conformance.invalid.evil.test".into())]);
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
        .instantiate(
            &component,
            manifest.clone(),
            Policy::deny_all(),
            Services::none(),
        )
        .expect("first instance");
    first
        .on_event(Event::Clicked(NodeId::named("spin")))
        .expect("event");
    let _ = first.view();
    assert!(first.is_dead());

    let mut second = runtime
        .instantiate(&component, manifest, Policy::deny_all(), Services::none())
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

#[test]
fn a_measurement_crosses_the_boundary_with_the_role_the_widget_named() {
    // The role used to cross as a `u8` index into an enum, which nothing
    // stopped a widget getting wrong. It is the enum now, and this is the
    // assertion that the value survives the trip: the ruler answers 20 pixels
    // per character for `Display` and 7 for everything else, and the widget
    // asks for `Body`.
    let scratch = Scratch::new("role");
    let (_runtime, mut instance) = start_with(everything(), None, served(&scratch));
    // `measured` compares "MMMMMMMMMM" against "i" and "", so a host that
    // ignored the text would fail before the role ever mattered.
    assert_eq!(
        outcome(&instance.view().expect("render"), "measure"),
        "measured"
    );
}

#[test]
fn a_widget_reading_the_machines_load_gets_a_fraction() {
    // `cpu_load` is the one served capability whose answer nobody can predict,
    // so what is asserted is the range — and that asking twice in one view
    // does not block, which is the invariant a sampling implementation gets
    // wrong by sleeping.
    let scratch = Scratch::new("metrics");
    let (_runtime, mut instance) = start_with(everything(), None, served(&scratch));

    let began = std::time::Instant::now();
    let view = instance.view().expect("render");
    let took = began.elapsed();

    // On a platform with no counters wired up the honest answer is a refusal,
    // and the widget reports it as one rather than as a zero.
    let answer = outcome(&view, "metrics");
    assert!(
        answer == "allowed" || answer == "unavailable",
        "cpu_load answered {answer:?}"
    );
    assert!(
        took < Duration::from_millis(100),
        "reading a counter took {took:?}; a host function may not block"
    );
}

#[test]
fn the_abi_version_is_checked_before_anything_else_runs() {
    // The ABI has moved three times while the framework was pre-release:
    // `read-file` answering `ok(none)`, `measure-text` taking a role rather
    // than a `u8`, and one asynchronous mechanism replacing one per
    // capability. The check is what makes that safe: a widget built against an
    // older world is refused at instantiation, in a sentence naming both
    // numbers, rather than failing somewhere confusing later.
    let runtime = Runtime::new().expect("runtime");
    let component = runtime.compile(&component_bytes()).expect("compile");
    let manifest = Arc::new(manifest());

    // The real widget matches, which is the case that has to keep working.
    runtime
        .instantiate(&component, manifest, Policy::deny_all(), Services::none())
        .expect("a widget built from this tree speaks this ABI");
    assert_eq!(uniwidged_runtime::manifest::ABI_VERSION, 4);
}

#[test]
fn an_asynchronous_request_finishes_and_comes_back_by_its_token() {
    // The whole of the asynchronous path, end to end and through the sandbox:
    // the widget starts a command and is not blocked, the work happens
    // somewhere else, the host learns which token finished, delivers an event
    // carrying it, and the widget collects the answer by that token.
    //
    // Nothing in the project exercised this before — `Event::Delivered` was a
    // variant nothing produced.
    let scratch = Scratch::new("command");
    let tasks = Arc::new(Tasks::default());
    let (_runtime, mut instance) = start_with(
        everything(),
        None,
        served_with_tasks(&scratch, tasks.clone()),
    );

    let began = std::time::Instant::now();
    assert_eq!(
        outcome(&instance.view().expect("render"), "command"),
        "started"
    );
    assert!(
        began.elapsed() < Duration::from_millis(200),
        "starting a command took {:?}, so the widget waited for it",
        began.elapsed()
    );

    // The host's job: notice, and tell the widget which token it was. The
    // widget has two requests outstanding — this one and a fetch — so tokens
    // arrive in whatever order they finish, exactly as on a real desktop.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        for (owner, token) in tasks.finished() {
            assert_eq!(owner, uniwidged_core::WidgetId(1), "somebody else's task");
            instance
                .on_event(Event::Delivered(token))
                .expect("the event is delivered");
        }
        let said = outcome(&instance.view().expect("render"), "command");
        if said != "started" && said != "pending" {
            assert_eq!(
                said, "conformance",
                "which is what `echo conformance` says, so the answer really is \
                 that command's output"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the command never came back"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_command_the_manifest_did_not_declare_is_refused() {
    // `command:echo` is a grant for `echo`. The conformance widget runs
    // exactly that, so granting a *different* command must refuse the same
    // call the test above allowed.
    let scratch = Scratch::new("wrong-command");
    let policy = Policy::from_granted([Capability::Command("uptime".into())]);
    let (_runtime, mut instance) = start_with(policy, None, served(&scratch));
    assert_eq!(
        outcome(&instance.view().expect("render"), "command"),
        "not-granted"
    );
}

#[test]
fn a_widget_ignores_an_answer_it_was_not_waiting_for() {
    // Two things guard this, and they are different. `Tasks::take` refuses a
    // token that is not the widget's — covered in `tasks`, where a widget
    // guessing its neighbour's number is the case. This is the other one: a
    // token the widget was handed but never asked for must not be acted on,
    // because a widget with several requests in flight would otherwise show
    // one answer in another's place.
    let scratch = Scratch::new("stranger");
    let tasks = Arc::new(Tasks::default());
    let (_runtime, mut instance) = start_with(
        everything(),
        None,
        served_with_tasks(&scratch, tasks.clone()),
    );

    // A task belonging to somebody else entirely. Handed to this widget
    // directly, which is what a host bug would look like — and which the
    // ownership check has to survive anyway.
    let stranger = tasks
        .start(
            uniwidged_core::WidgetId(99),
            Box::new(|| uniwidged_runtime::tasks::TaskResult::Failed("not yours".into())),
        )
        .expect("started");
    wait_for_a_task(&tasks);

    // Twice: the first view *starts* the command and says so, and every one
    // after it says the request is outstanding. The baseline has to be the
    // steady state, or this test measures the widget working.
    instance.view().expect("render");
    let before = outcome(&instance.view().expect("render"), "command");
    assert_eq!(before, "pending", "a request is outstanding");

    instance
        .on_event(Event::Delivered(stranger))
        .expect("the event is delivered");
    assert_eq!(
        outcome(&instance.view().expect("render"), "command"),
        before,
        "a token this widget never started must change nothing"
    );
}

/// Waits until at least one task has been reported finished.
fn wait_for_a_task(tasks: &Tasks) -> Vec<(uniwidged_core::WidgetId, u32)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let finished = tasks.finished();
        if !finished.is_empty() {
            return finished;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no task finished within fifteen seconds"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
