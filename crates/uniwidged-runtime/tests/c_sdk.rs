//! A widget written in C, run by the real runtime.
//!
//! The interface is a WIT file, and the claim that it is a *contract* rather
//! than a Rust API with extra steps is only as good as a second language that
//! has been held to it. This builds `sdks/c/examples/counter` with wasi-sdk and
//! `wit-bindgen c`, instantiates it under a deny-all policy, and checks the
//! three things every widget does:
//!
//! * **it describes a view the host accepts** — the tree crossed the canonical
//!   ABI as an arena built by hand in C, and the host's validation (indices in
//!   range, children after parents, one root) passed it;
//! * **it reads its own settings** — the value the user set is the value the
//!   C code counts from;
//! * **it answers a click by id** — the id the C SDK hashed is the id the Rust
//!   side's `NodeId::named` hashes, so the click lands.
//!
//! It needs wasi-sdk (`WASI_SDK_PATH`, default `/opt/wasi-sdk`), `make` and
//! `wit-bindgen` 0.61. Without them it skips, unless `UNIWIDGED_REQUIRE_C_SDK`
//! is set, which is what CI does.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use uniwidged_core::NodeId;
use uniwidged_runtime::capability::Policy;
use uniwidged_runtime::instance::{Event, Runtime};
use uniwidged_runtime::manifest::Manifest;
use uniwidged_runtime::services::Services;
use uniwidged_runtime::settings::{SettingValue, Settings};
use uniwidged_view::{Content, Node};

fn sdk_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdks/c")
}

/// Builds the example into a directory of this test's own, or says why not.
fn build() -> Result<(Vec<u8>, Manifest), String> {
    let build = std::env::temp_dir().join(format!("uniwidged-c-sdk-{}", std::process::id()));
    let output = Command::new("make")
        .current_dir(sdk_dir())
        .arg(format!("BUILD={}", build.display()))
        .output()
        .map_err(|e| format!("make could not be run: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "the C example did not build:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let wasm = std::fs::read(build.join("c-counter.wasm")).map_err(|e| e.to_string())?;
    let manifest =
        std::fs::read_to_string(build.join("c-counter.toml")).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&build);
    Ok((wasm, Manifest::parse(&manifest).map_err(|e| e.to_string())?))
}

fn texts(node: &Node, into: &mut Vec<String>) {
    if let Content::Text(text) = &node.content {
        into.push(text.clone());
    }
    for child in &node.children {
        texts(child, into);
    }
}

fn find(node: &Node, id: NodeId) -> Option<&Node> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, id))
}

#[test]
fn a_widget_written_in_c_renders_reads_its_settings_and_counts_clicks() {
    let (wasm, manifest) = match build() {
        Ok(built) => built,
        Err(why) if std::env::var_os("UNIWIDGED_REQUIRE_C_SDK").is_none() => {
            eprintln!("skipping: {why}");
            return;
        }
        Err(why) => panic!("{why}"),
    };
    assert!(
        manifest.capabilities.is_empty(),
        "the example is meant to need nothing"
    );

    // The user said to start from 41. Every other declared key at its default,
    // which is what `Host::install` hands a widget.
    let settings = Settings::new(
        manifest
            .settings
            .iter()
            .map(|setting| {
                let value = if setting.key == "start" {
                    SettingValue::Number(41.0)
                } else {
                    setting.default.clone()
                };
                (setting.key.clone(), value)
            })
            .collect(),
    );

    let runtime = Runtime::new().expect("a runtime");
    let component = runtime.compile(&wasm).expect("the C component compiles");
    let mut instance = runtime
        .instantiate(
            &component,
            Arc::new(manifest),
            Policy::deny_all(),
            Services::none().with_settings(settings),
        )
        .expect("the C component instantiates, and its `start` succeeds");

    let view = instance.view().expect("a view the host accepts");
    assert_eq!(
        view.root.id,
        NodeId::named("counter"),
        "the root is the panel"
    );
    let mut seen = Vec::new();
    texts(&view.root, &mut seen);
    assert!(
        seen.iter().any(|t| t == "41"),
        "the setting did not reach the C code: {seen:?}"
    );
    let plus = find(&view.root, NodeId::named("plus"))
        .expect("the button, under the id the Rust SDK would give it");
    assert_eq!(plus.semantics.label.as_deref(), Some("Count one more"));

    assert!(
        instance
            .on_event(Event::Clicked(NodeId::named("plus")))
            .expect("the click is handled"),
        "a click on the button changes the view"
    );
    assert!(
        !instance
            .on_event(Event::Clicked(NodeId::named("elsewhere")))
            .expect("a click elsewhere is handled"),
        "a click on nothing changes nothing"
    );
    let view = instance.view().expect("a second view");
    let mut seen = Vec::new();
    texts(&view.root, &mut seen);
    assert!(
        seen.iter().any(|t| t == "42"),
        "the click was not counted: {seen:?}"
    );
    assert_eq!(
        instance.next_wake().expect("a wake"),
        uniwidged_core::Wake::Never,
        "a widget that defines no wake is never woken"
    );
}
