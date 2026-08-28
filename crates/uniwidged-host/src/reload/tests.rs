//! What a reload does to a desktop that is already running.
//!
//! Unit tests, against a host with no widgets in it: what is being checked
//! here is the *diff* — which names are new, which are gone, which are
//! disabled — and that is decided before any widget is touched. The half that
//! needs real components, where a setting actually reaches a running widget
//! and the widget says its view differs, is in `tests/desktop.rs`, where there
//! are real components to reach.

use super::*;
use crate::config::{Config, Placement, WidgetConfig};
use uniwidged_compose::Theme;
use uniwidged_runtime::manifest::Manifest;

fn host() -> Host {
    Host::new(Theme::default(), std::time::Duration::from_millis(16)).expect("a host")
}

fn entry(name: &str, enabled: bool) -> WidgetConfig {
    let manifest =
        Manifest::parse(&format!("name = \"{name}\"\nversion = \"1.0.0\"")).expect("a manifest");
    let mut config = WidgetConfig::granting_all(name, format!("{name}.wasm").into(), &manifest);
    config.enabled = enabled;
    config.placement = Placement::default();
    config
}

fn config(entries: Vec<WidgetConfig>) -> Config {
    Config { widgets: entries }
}

#[test]
fn a_widget_in_the_file_and_not_running_is_named_as_needing_a_restart() {
    // Installing needs a component compiled, a window created, a swapchain
    // attached and a placement resolved. Doing half of that on a signal is how
    // a desktop ends up with a window nothing draws into — so it is reported
    // rather than faked.
    let mut host = host();
    let applied = apply(&mut host, &config(vec![entry("clock", true)]));
    assert_eq!(applied.added, vec!["clock"]);
    assert!(applied.removed.is_empty());
    assert!(applied.needs_restart());
    assert!(!applied.is_quiet());
}

#[test]
fn a_widget_disabled_in_the_file_is_named_rather_than_stopped() {
    // Same argument in the other direction, and the file says something
    // different from "not there": a disabled widget keeps its placement, its
    // grants and its data, and the user expects to find it again.
    let mut host = host();
    // Nothing is running, so a disabled entry is neither added nor removed:
    // there is nothing to stop and nothing to start.
    let applied = apply(&mut host, &config(vec![entry("clock", false)]));
    assert!(applied.added.is_empty(), "{applied:?}");
    assert!(applied.removed.is_empty(), "{applied:?}");
    assert!(applied.is_quiet());
}

#[test]
fn an_empty_file_against_an_empty_desktop_changes_nothing() {
    // The common case by a wide margin: a reload fires whenever anything in
    // the configuration is touched, and most of those touches are about one
    // widget. A reload that reported work it did not do would make the log
    // useless.
    let mut host = host();
    let applied = apply(&mut host, &Config::default());
    assert_eq!(applied, Applied::default());
    assert!(applied.is_quiet());
    assert!(!applied.needs_restart());
}

#[test]
fn a_report_says_which_kind_of_change_it_is() {
    // `needs_restart` is what the caller prints a sentence about, and it must
    // not fire for a change that was applied. A user told to restart after a
    // setting already took effect would restart for nothing and learn to
    // ignore the message.
    let applied = Applied {
        reconfigured: vec![WidgetId(1)],
        ..Applied::default()
    };
    assert!(!applied.needs_restart());
    assert!(!applied.is_quiet());

    let complained = Applied {
        complaints: vec!["`clock`: `city` is set and this widget does not declare it".into()],
        ..Applied::default()
    };
    assert!(!complained.needs_restart());
    assert!(!complained.is_quiet(), "a complaint is worth printing");
}
