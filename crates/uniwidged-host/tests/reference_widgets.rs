//! The widgets somebody else is meant to copy.
//!
//! `widgets/clock`, `widgets/system-load`, `widgets/counter`, `widgets/uptime`
//! and `widgets/vault` are the framework's worked examples, one per served
//! capability. Examples rot, and a
//! rotted example is worse than none: it is the first thing a new author reads
//! and the last thing anybody runs. So they are built here, installed under
//! **exactly the capabilities their own manifests declare**, and asserted to
//! work.
//!
//! That last part is the point. A reference widget that only works because the
//! test granted it something its manifest does not ask for is a reference for
//! nothing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use uniwidged_compose::Theme;
use uniwidged_core::{NodeId, Point, Size, WidgetId};
use uniwidged_host::Host;
use uniwidged_runtime::capability::Policy;
use uniwidged_runtime::manifest::Manifest;
use uniwidged_runtime::services::Locale;

const SIZE: Size = Size {
    width: 240.0,
    height: 160.0,
};

fn widgets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../widgets")
}

fn component(name: &str) -> Vec<u8> {
    let dir = widgets_dir().join(name);
    let status = Command::new(env!("CARGO"))
        .current_dir(&dir)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .status()
        .expect("cargo should be runnable");
    assert!(
        status.success(),
        "could not build the {name} widget. The wasm32-wasip2 target is \
         required: rustup target add wasm32-wasip2"
    );
    let wasm = dir.join(format!(
        "target/wasm32-wasip2/release/{}_widget.wasm",
        name.replace('-', "_")
    ));
    std::fs::read(&wasm).unwrap_or_else(|e| panic!("{}: {e}", wasm.display()))
}

fn manifest(name: &str) -> Manifest {
    let text =
        std::fs::read_to_string(widgets_dir().join(name).join("widget.toml")).expect("widget.toml");
    Manifest::parse(&text).expect("a reference widget's manifest should parse")
}

/// A directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "uniwidged-reference-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A desktop with somewhere to write and a fixed locale, so what a widget
/// renders is the same wherever this runs.
fn desktop(data_root: &Path) -> Host {
    let mut host = Host::new(Theme::default(), Duration::from_millis(16)).expect("a host");
    host.set_data_root(data_root);
    // The keychain goes beside the data root rather than inside it: on a real
    // machine it sits with the configuration, and one widget's directory is
    // not somewhere another widget's secrets belong.
    host.set_keyring(std::sync::Arc::new(
        uniwidged_runtime::keyring::FileKeyring::new(data_root.join("..")),
    ));
    host.set_locale(Locale::default());
    host
}

/// Installs a widget with **exactly** what its manifest asks for.
fn install(host: &mut Host, name: &str) -> WidgetId {
    let manifest = manifest(name);
    let policy = Policy::from_granted(manifest.capabilities.clone());
    let id = host
        .install(&component(name), manifest, policy)
        .expect("install");
    host.attach_readback(id, SIZE).expect("surface");
    id
}

fn tick(host: &mut Host) {
    host.tick(Instant::now(), SystemTime::now()).expect("tick");
}

/// Clicks a named node through the host's own hit-testing.
///
/// Through the compose pass's rectangles at a point inside the button, rather
/// than by handing the widget a node id: what these tests are for is the whole
/// path, and a test that shortcut the hit-testing would keep passing after the
/// buttons stopped being where the layout puts them.
fn click(host: &mut Host, id: WidgetId, node: &str) {
    let button = host
        .composed(id)
        .expect("a composition")
        .hit_rects
        .get(&NodeId::named(node))
        .copied()
        .unwrap_or_else(|| panic!("`{node}` should be on screen"));
    let hit = host
        .click(id, Point::new(button.center().x, button.center().y))
        .expect("click");
    assert_eq!(hit.node(), Some(NodeId::named(node)));
}

/// The text of a named node in a widget's current view.
fn text_of(host: &Host, id: WidgetId, node: &str) -> String {
    fn find(node: &uniwidged_view::Node, id: NodeId) -> Option<&uniwidged_view::Node> {
        if node.id == id {
            return Some(node);
        }
        node.children.iter().find_map(|c| find(c, id))
    }
    let view = host
        .widget(id)
        .and_then(|w| w.last_view())
        .expect("the widget should have rendered");
    let found = find(&view.root, NodeId::named(node))
        .unwrap_or_else(|| panic!("no node named {node} in {view:?}"));
    match &found.content {
        uniwidged_view::Content::Text(text) => text.clone(),
        other => panic!("expected text at {node}, got {other:?}"),
    }
}

#[test]
fn the_clock_shows_a_time_with_only_the_capabilities_it_asks_for() {
    // `timer` and `locale`, and nothing else. A clock that needed storage or
    // the network would be teaching the wrong lesson.
    let scratch = Scratch::new("clock");
    let mut host = desktop(&scratch.0);
    let id = install(&mut host, "clock");
    tick(&mut host);

    let time = text_of(&host, id, "time");
    assert_eq!(time.len(), 5, "a 24-hour clock reads HH:MM, got {time:?}");
    assert_eq!(time.as_bytes()[2], b':', "{time:?}");
    assert!(
        time.chars().filter(char::is_ascii_digit).count() == 4,
        "{time:?}"
    );

    let date = text_of(&host, id, "date");
    assert_eq!(date.len(), 10, "an ISO date reads YYYY-MM-DD, got {date:?}");
    assert!(
        !date.contains("not granted"),
        "the clock was granted `locale` and should have used it"
    );

    // And it asks to be woken at a whole minute rather than on a delay, which
    // is what lets eight clocks cost one wake-up.
    assert!(
        host.next_timeout(Instant::now())
            .is_some_and(|d| d <= Duration::from_secs(60)),
        "a clock schedules itself"
    );
}

#[test]
fn the_meter_reads_the_machine_and_stays_in_range() {
    let scratch = Scratch::new("load");
    let mut host = desktop(&scratch.0);
    let id = install(&mut host, "system-load");
    tick(&mut host);

    let reading = text_of(&host, id, "reading");
    if reading == "—" {
        // A platform with no counters. The widget says so rather than showing
        // a zero, which is the behaviour under test on such a machine.
        return;
    }
    let percent: u32 = reading
        .trim_end_matches('%')
        .parse()
        .unwrap_or_else(|_| panic!("a meter should read a percentage, got {reading:?}"));
    assert!(percent <= 100, "{percent}% is not a fraction of a machine");
}

#[test]
fn the_counter_remembers_across_a_restart() {
    // The property the whole supervisor design rests on: a widget's state is
    // derived from its data, so a fresh instance is a working widget again.
    // Here that is demonstrated rather than argued — two hosts, one directory.
    let scratch = Scratch::new("counter");

    let clicks = {
        let mut host = desktop(&scratch.0);
        let id = install(&mut host, "counter");
        tick(&mut host);
        assert_eq!(
            text_of(&host, id, "count"),
            "0",
            "a first run starts at zero"
        );

        // Through the host's own hit-testing, at a point inside the button —
        // the widget is told *which node*, and this test does not get to
        // shortcut that.
        for _ in 0..3 {
            let composed = host.composed(id).expect("a composition");
            let button = composed
                .hit_rects
                .get(&NodeId::named("increment"))
                .copied()
                .expect("the Count button should be on screen");
            let hit = host
                .click(id, Point::new(button.center().x, button.center().y))
                .expect("click");
            assert_eq!(hit.node(), Some(NodeId::named("increment")));
            tick(&mut host);
        }
        assert_eq!(text_of(&host, id, "count"), "3");
        assert!(
            host.widget(id)
                .and_then(|w| w.last_view())
                .is_some_and(|v| !format!("{v:?}").contains("not saved")),
            "the counter was granted `fs:state` and should have saved"
        );
        3
    };

    // A completely new desktop over the same data directory: new process
    // state, new instances, nothing carried over but the files.
    let mut host = desktop(&scratch.0);
    let id = install(&mut host, "counter");
    tick(&mut host);
    assert_eq!(
        text_of(&host, id, "count"),
        clicks.to_string(),
        "the count is on disk, so a restart finds it"
    );

    // And it landed where the manifest said it would: inside `state/`, in the
    // widget's own directory, and nowhere else.
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("counter/state/count")).expect("the saved count"),
        "3"
    );
}

#[test]
fn every_reference_widget_declares_only_what_it_uses() {
    // A worked example asking for more than it needs teaches every reader to
    // over-declare, and over-declaring is the failure mode a capability system
    // exists to prevent. The list is written out rather than derived so that
    // adding a capability to a reference widget has to be a deliberate edit
    // here too.
    for (name, expected) in [
        ("clock", vec!["timer", "locale"]),
        ("system-load", vec!["timer", "metrics"]),
        ("counter", vec!["fs:state"]),
        ("uptime", vec!["timer", "command:uptime"]),
        ("vault", vec!["secrets"]),
    ] {
        let declared: Vec<String> = manifest(name)
            .capabilities
            .iter()
            .map(uniwidged_host::config::capability_text)
            .collect();
        assert_eq!(declared, expected, "{name}");
    }
}

#[test]
fn the_uptime_widget_runs_a_command_and_shows_what_it_said() {
    // The asynchronous capability, from the outside: the widget's first frame
    // says it is waiting, an answer arrives on a later turn, and the frame
    // after that has the machine's real uptime in it.
    //
    // Skipped where `uptime` is not installed, because the widget's job here is
    // to demonstrate the *mechanism* and a missing binary would only be
    // demonstrating the failure path — which its own unit tests already cover.
    if std::process::Command::new("uptime")
        .arg("-p")
        .output()
        .is_err()
    {
        eprintln!("skipping: no `uptime` on this machine");
        return;
    }

    let scratch = Scratch::new("uptime");
    let mut host = desktop(&scratch.0);
    let id = install(&mut host, "uptime");
    tick(&mut host);
    assert_eq!(
        text_of(&host, id, "reading"),
        "…",
        "the first frame is drawn before the answer exists, and says so"
    );

    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        tick(&mut host);
        if text_of(&host, id, "reading") != "…" {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let reading = text_of(&host, id, "reading");
    assert!(
        reading.starts_with("up "),
        "`uptime -p` says how long it has been up, got {reading:?}"
    );
    assert!(
        host.widget(id)
            .and_then(|w| w.last_view())
            .is_some_and(|v| !format!("{v:?}").contains("trouble")),
        "nothing went wrong"
    );
}

#[test]
fn the_vault_signs_in_and_out_and_remembers_across_a_restart() {
    // The reference widget for `secrets`, and the same demonstration the
    // counter makes for `fs`: a widget's state is derived from what it stored,
    // so a restart is cheap. For a credential that matters more than for a
    // number — a widget that forgets its token on every restart puts a sign-in
    // screen where a widget was.
    let scratch = Scratch::new("vault");

    {
        let mut host = desktop(&scratch.0.join("widgets"));
        let id = install(&mut host, "vault");
        tick(&mut host);
        assert_eq!(
            text_of(&host, id, "state"),
            "not signed in",
            "a first run has nothing stored, and that is not an error"
        );

        click(&mut host, id, "store");
        tick(&mut host);
        assert_eq!(
            text_of(&host, id, "state"),
            "signed in",
            "the vault was granted `secrets` and should have used it"
        );
    }

    // A whole new desktop over the same machine — which is what a log-out is.
    {
        let mut host = desktop(&scratch.0.join("widgets"));
        let id = install(&mut host, "vault");
        tick(&mut host);
        assert_eq!(
            text_of(&host, id, "state"),
            "signed in",
            "the credential should have outlived the desktop that stored it"
        );

        click(&mut host, id, "forget");
        tick(&mut host);
        assert_eq!(text_of(&host, id, "state"), "not signed in");
    }

    // And signing out is durable too: a widget that appeared to sign out and
    // came back signed in would be the worse half of the same bug.
    let mut host = desktop(&scratch.0.join("widgets"));
    let id = install(&mut host, "vault");
    tick(&mut host);
    assert_eq!(text_of(&host, id, "state"), "not signed in");
}

#[test]
fn the_vault_keeps_the_credential_out_of_the_view_and_off_the_disk_in_the_clear() {
    // Two places a secret leaks without anybody meaning it to. The view is
    // composed, hit-tested and handed to an accessibility tree, so a secret in
    // it is a secret a screen reader reads aloud. The store is a file in
    // somebody's configuration directory, so a secret in it in the clear is a
    // secret in their backups.
    let scratch = Scratch::new("vault-quiet");
    let mut host = desktop(&scratch.0.join("widgets"));
    let id = install(&mut host, "vault");
    tick(&mut host);
    click(&mut host, id, "store");
    tick(&mut host);

    let view = host.widget(id).and_then(|w| w.last_view()).expect("a view");
    let rendered = format!("{view:?}");
    assert!(
        !rendered.contains("a token a real widget"),
        "the credential reached the view"
    );

    let store = std::fs::read(scratch.0.join("keyring.enc")).expect("the keychain");
    assert!(
        !store.windows(5).any(|w| w == b"token"),
        "the credential is on disk in the clear"
    );
}

#[test]
fn a_setting_a_widget_does_not_redraw_for_is_still_applied() {
    // The distinction a live desktop found and no unit test had. `system-load`
    // colours its meter by comparing a reading against two thresholds, so
    // moving a threshold past nothing changes its accent and it correctly
    // answers that its view is unchanged — while its configuration has very
    // much changed.
    //
    // A caller that read "the view is unchanged" as "nothing was applied"
    // printed `nothing changed` over a setting it had just applied, which is
    // the message that makes somebody change the value again.
    let scratch = Scratch::new("applied");
    let mut host = desktop(&scratch.0.join("widgets"));
    let id = install(&mut host, "system-load");
    tick(&mut host);

    let report = host
        .configure(
            id,
            &std::collections::BTreeMap::from([(
                "warn-at".to_string(),
                uniwidged_runtime::settings::SettingValue::Number(0.9),
            )]),
        )
        .expect("configure");
    assert!(report.applied, "0.9 is not the declared default of 0.6");
    assert!(report.complaints.is_empty(), "{report:?}");

    // Asking again with the same value is the case that really is nothing:
    // saving a form usually changes nothing, and every widget on the desktop
    // must not pay a call and a tree comparison for it.
    let again = host
        .configure(
            id,
            &std::collections::BTreeMap::from([(
                "warn-at".to_string(),
                uniwidged_runtime::settings::SettingValue::Number(0.9),
            )]),
        )
        .expect("configure");
    assert!(!again.applied);
    assert!(!again.redraws);
}
