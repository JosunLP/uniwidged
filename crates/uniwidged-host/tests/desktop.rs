//! A desktop, running.
//!
//! Real WASM widgets, the real runtime, the real compositor and the real
//! renderer — everything but a window, which the M0 and M4 spikes show
//! produces the same pixels either way.
//!
//! The tests these exist for are the ones about *cost*: that an idle desktop
//! submits nothing, that a widget which has nothing new to say is not drawn,
//! and that a broken widget stops costing anything without leaving a hole in
//! the screen.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use uniwidged_compose::Theme;
use uniwidged_core::{NodeId, Point, Size, WidgetId};
use uniwidged_host::config::{Config, WidgetConfig};
use uniwidged_host::supervisor::State;
use uniwidged_host::{Host, TickReport};
use uniwidged_runtime::capability::{Capability, Policy};
use uniwidged_runtime::instance::Event;
use uniwidged_runtime::limits::Limits;
use uniwidged_runtime::manifest::Manifest;

const SIZE: Size = Size {
    width: 240.0,
    height: 160.0,
};

fn widgets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../widgets")
}

/// Builds one of the repository's widgets and returns its component bytes.
///
/// Built rather than looked for, exactly as the conformance suite does: a test
/// that skips when an artefact is missing eventually always skips.
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
    Manifest::parse(&text).expect("the manifest should parse")
}

fn host() -> Host {
    Host::new(Theme::default(), Duration::from_millis(16)).expect("a host")
}

/// Installs a widget and gives it somewhere to draw.
fn install(host: &mut Host, name: &str, policy: Policy) -> WidgetId {
    let id = host
        .install(&component(name), manifest(name), policy)
        .expect("install");
    host.attach_readback(id, SIZE).expect("surface");
    id
}

fn now() -> (Instant, SystemTime) {
    (Instant::now(), SystemTime::now())
}

/// Ticks until something is redrawn, or gives up.
fn tick_until_redraw(host: &mut Host, within: Duration) -> TickReport {
    let deadline = Instant::now() + within;
    loop {
        let (i, w) = now();
        let report = host.tick(i, w).expect("tick");
        if !report.redrawn.is_empty() || Instant::now() > deadline {
            return report;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_widget_draws_its_first_frame_without_being_asked() {
    // A widget that answers `Wake::Never` still has to appear. Its surface is
    // empty until it does, and "never" is about updates, not about existing.
    let mut host = host();
    let id = install(&mut host, "conformance", Policy::deny_all());
    assert_eq!(host.next_timeout(Instant::now()), Some(Duration::ZERO));

    let (i, w) = now();
    let report = host.tick(i, w).expect("tick");
    assert_eq!(report.redrawn, vec![id]);
    assert!(report.submits > 0);
    assert!(host.widget(id).unwrap().last_view().is_some());
}

#[test]
fn an_idle_desktop_submits_nothing() {
    // The single most important assertion in the project, at the level a user
    // would feel it. Eight widgets, none of them scheduled, and the loop
    // blocks rather than polling.
    let mut host = host();
    let wasm = component("conformance");
    let manifest = manifest("conformance");
    for _ in 0..8 {
        let id = host
            .install(&wasm, manifest.clone(), Policy::deny_all())
            .expect("install");
        host.attach_readback(id, SIZE).expect("surface");
    }

    let (i, w) = now();
    let first = host.tick(i, w).expect("tick");
    assert_eq!(first.redrawn.len(), 8, "the first frames");

    // And then nothing, for as long as anyone cares to wait.
    assert_eq!(
        host.next_timeout(Instant::now()),
        None,
        "an idle desktop must block, not poll"
    );
    for _ in 0..5 {
        let (i, w) = now();
        let report = host.tick(i, w).expect("tick");
        assert!(
            report.is_quiet(),
            "an idle desktop drew something: {report:?}"
        );
        assert!(report.ran.is_empty(), "an idle widget was run: {report:?}");
    }
}

#[test]
fn a_widget_with_nothing_new_to_say_is_not_drawn() {
    // The other half of the cost model: a widget can be *running* — scheduled,
    // asked, answering — and still cost no GPU work, because its view has not
    // changed. Holding the ticker is exactly that state.
    let mut host = host();
    let id = install(
        &mut host,
        "ticker",
        Policy::from_granted([Capability::Timer]),
    );
    tick_until_redraw(&mut host, Duration::from_secs(2));

    let hold = host
        .composed(id)
        .and_then(|c| c.hit_rects.get(&NodeId::named("hold")).copied())
        .expect("the hold chip should have a rectangle");
    host.click(id, hold.center()).expect("click");
    tick_until_redraw(&mut host, Duration::from_secs(1));

    let (mut ran, mut redrawn) = (0, 0);
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        let (i, w) = now();
        let report = host.tick(i, w).expect("tick");
        ran += report.ran.len();
        redrawn += report.redrawn.len();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        ran > 2,
        "the held widget should still be scheduled and asked"
    );
    assert_eq!(
        redrawn, 0,
        "a widget with an unchanged view must not be drawn"
    );
}

#[test]
fn a_widget_that_changes_is_drawn_again() {
    let mut host = host();
    let id = install(
        &mut host,
        "ticker",
        Policy::from_granted([Capability::Timer]),
    );
    let first = tick_until_redraw(&mut host, Duration::from_secs(2));
    assert_eq!(first.redrawn, vec![id]);
    let second = tick_until_redraw(&mut host, Duration::from_secs(2));
    assert_eq!(second.redrawn, vec![id], "the ticker should keep changing");
    assert!(second.submits > 0);
}

#[test]
fn a_click_reaches_the_widget_as_a_node_not_a_coordinate() {
    // The host did the layout, so the host hit-tests. A widget is told *which
    // node*, which is what lets it be written without any notion of where
    // anything is.
    let mut host = host();
    let id = install(&mut host, "conformance", Policy::deny_all());
    let (i, w) = now();
    host.tick(i, w).expect("tick");

    let target = NodeId::named("locale");
    let rect = host
        .composed(id)
        .and_then(|c| c.hit_rects.get(&target).copied())
        .expect("the row should have a rectangle");
    assert_eq!(host.click(id, rect.center()).expect("click"), Some(target));
    assert_eq!(
        host.click(id, Point::new(-10.0, -10.0)).expect("click"),
        None,
        "a click outside the widget hits nothing"
    );
}

#[test]
fn a_click_that_changes_something_schedules_a_frame() {
    let mut host = host();
    let id = install(&mut host, "conformance", Policy::deny_all());
    let (i, w) = now();
    host.tick(i, w).expect("tick");
    assert_eq!(host.next_timeout(Instant::now()), None, "then quiet");

    let rect = host
        .composed(id)
        .and_then(|c| c.hit_rects.get(&NodeId::named("net")).copied())
        .expect("a rectangle");
    host.click(id, rect.center()).expect("click");
    assert_eq!(
        host.next_timeout(Instant::now()),
        Some(Duration::ZERO),
        "a widget that says its view changed should be due at once"
    );
    let (i, w) = now();
    assert_eq!(host.tick(i, w).expect("tick").redrawn, vec![id]);
}

#[test]
fn a_widget_that_hangs_once_is_restarted_and_recovers() {
    // The cheerful half, and it was a surprise: a fresh instance does not
    // carry the state that made the old one hang, so restarting *cures* a
    // state-dependent hang. The first version of this test asserted the
    // opposite and was wrong about how the system behaves.
    let mut host = host();
    let mut manifest = manifest("conformance");
    manifest.limits = Limits {
        call_deadline: Duration::from_millis(15),
        overruns_before_suspend: 3,
        ..Limits::default()
    };
    let id = host
        .install(&component("conformance"), manifest, Policy::deny_all())
        .expect("install");
    host.attach_readback(id, SIZE).expect("surface");
    let (i, w) = now();
    host.tick(i, w).expect("first frame");

    // "spin" is not a node the widget draws, so this cannot come from a click.
    // A hotkey or a menu item would arrive the same way.
    host.deliver(id, Event::Clicked(NodeId::named("spin")))
        .expect("deliver");

    let (i, w) = now();
    let died = host.tick(i, w).expect("tick");
    assert_eq!(
        died.restarting,
        vec![id],
        "the hang should end the instance"
    );

    // The backoff has to pass before it is tried again.
    std::thread::sleep(Duration::from_millis(250));
    let (i, w) = now();
    let recovered = host.tick(i, w).expect("tick");
    assert_eq!(recovered.ran, vec![id]);
    assert!(
        recovered.restarting.is_empty() && recovered.suspended.is_empty(),
        "a fresh instance has no reason to hang: {recovered:?}"
    );
    assert!(matches!(host.widget(id).unwrap().state(), State::Running));
    // Not redrawn, and that is right: a fresh instance starts from nothing, so
    // its view is the one that was already on screen. The recovery is visible
    // in the state, not in a frame.
    assert!(recovered.redrawn.is_empty(), "{recovered:?}");
}

#[test]
fn a_widget_that_always_hangs_is_given_up_on_and_stops_costing_anything() {
    // The half that has to be contained. `widgets/broken` hangs on every
    // instance, so restarting cannot help — and restarting a bug sixty times a
    // second is how a widget framework becomes the reason a laptop is warm.
    let mut host = host();
    let mut manifest = manifest("broken");
    manifest.limits = Limits {
        call_deadline: Duration::from_millis(15),
        overruns_before_suspend: 3,
        ..Limits::default()
    };
    let id = host
        .install(&component("broken"), manifest, Policy::deny_all())
        .expect("a broken widget still installs; it is only broken when run");
    host.attach_readback(id, SIZE).expect("surface");

    // A failed widget is scheduled with a backoff rather than retried at once,
    // so this waits rather than spinning — which is the behaviour being
    // tested as much as the count is.
    let mut restarts = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let (i, w) = now();
        let report = host.tick(i, w).expect("tick");
        restarts += report.restarting.len();
        if matches!(host.widget(id).unwrap().state(), State::Suspended(_)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        matches!(host.widget(id).unwrap().state(), State::Suspended(_)),
        "got {:?}",
        host.widget(id).unwrap().state()
    );
    assert!(
        (1..=3).contains(&restarts),
        "it should have been tried a few times and then given up on, not {restarts}"
    );

    // And then it costs nothing at all.
    assert_eq!(host.next_timeout(Instant::now()), None);
    let (i, w) = now();
    assert!(host.tick(i, w).expect("tick").is_quiet());
}

#[test]
fn a_theme_change_redraws_everything_without_any_view_changing() {
    // Appearance is not part of a widget's view, so the ordinary damage check
    // cannot see it. If this did not force a redraw, a light/dark switch would
    // leave every widget in the old theme until it happened to update.
    let mut host = host();
    let id = install(&mut host, "conformance", Policy::deny_all());
    let (i, w) = now();
    host.tick(i, w).expect("first frame");
    assert_eq!(host.next_timeout(Instant::now()), None);

    host.set_theme(Theme::light());
    assert_eq!(host.next_timeout(Instant::now()), Some(Duration::ZERO));
    let (i, w) = now();
    let report = host.tick(i, w).expect("tick");
    assert_eq!(report.redrawn, vec![id], "{report:?}");
}

#[test]
fn a_widget_reports_its_accessibility_tree_to_the_host() {
    let mut host = host();
    let id = install(&mut host, "conformance", Policy::deny_all());
    let (i, w) = now();
    host.tick(i, w).expect("tick");
    let nodes = host.accessibility();
    assert!(!nodes.is_empty());
    assert!(
        nodes
            .iter()
            .any(|(node_id, _)| *node_id == uniwidged_compose::a11y::root_id(id)),
        "the widget's root should be in the tree"
    );
}

#[test]
fn what_a_widget_logs_reaches_the_host() {
    let mut host = host();
    install(&mut host, "conformance", Policy::deny_all());
    let (i, w) = now();
    host.tick(i, w).expect("tick");
    let log = host.take_log();
    assert!(
        log.iter().any(|line| line.starts_with("[conformance]")),
        "got {log:?}"
    );
}

#[test]
fn a_configuration_round_trips_through_a_file() {
    // A desktop that survives a restart is a desktop whose configuration
    // survives being written and read.
    let dir = std::env::temp_dir().join("uniwidged-desktop-test");
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("config.toml");

    let manifest = manifest("ticker");
    let mut config = Config::default();
    config.widgets.push(WidgetConfig::granting_all(
        "ticker",
        PathBuf::from("widgets/ticker.wasm"),
        &manifest,
    ));
    config.widget_mut("ticker").unwrap().placement.x = 300;
    config.write(&path).expect("write");

    let again = Config::read(&path).expect("read");
    assert_eq!(again, config);
    assert_eq!(again.widget("ticker").unwrap().placement.x, 300);
    assert!(again.widget("ticker").unwrap().policy(&manifest).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}
