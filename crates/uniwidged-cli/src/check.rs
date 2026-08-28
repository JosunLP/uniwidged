//! `uniwidged check` — is this a widget, and who signed it?
//!
//! Everything `install` would find out, without installing anything: the
//! manifest parses, the component is a component, it speaks this host's ABI, it
//! starts, it draws — and here is what it will ask your users for.
//!
//! It exists because the alternative is finding out on somebody else's desktop.
//! A widget author runs it before publishing; a person runs it before trusting;
//! and a continuous-integration job runs it on every commit, which is what
//! `crates/uniwidged-cli/tests/scaffold.rs` does with the widget `uniwidged new`
//! writes.

use std::path::PathBuf;
use std::sync::Arc;

use uniwidged_runtime::capability::Policy;
use uniwidged_runtime::instance::Runtime;
use uniwidged_runtime::manifest::Manifest;
use uniwidged_runtime::services::Services;

use crate::trust::Provenance;

pub fn check(args: &[String]) -> Result<(), String> {
    let component = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or("which widget? `uniwidged check <widget.wasm>`")?;
    let component = PathBuf::from(component);
    let manifest_path = crate::flag(args, "--manifest")
        .map(PathBuf::from)
        .unwrap_or_else(|| component.with_extension("toml"));

    let (bytes, manifest_text) = crate::trust::read_widget(&component, &manifest_path)?;
    let manifest = Manifest::parse(&manifest_text).map_err(|e| e.to_string())?;

    println!("\n  {} {}", manifest.name, manifest.version);
    if !manifest.description.is_empty() {
        println!("  {}", manifest.description);
    }

    // The sandbox is what makes running somebody's code reasonable once it is
    // running. This is the question before that.
    match crate::trust::provenance(&component, &manifest_path)? {
        Provenance::Trusted(who) => println!("\n  signature: signed by {who}"),
        Provenance::Refused(why) => println!("\n  signature: REFUSED — {why}"),
        Provenance::Unsigned => println!("\n  signature: none"),
    }

    println!("\n  It will ask to:");
    for line in manifest.consent_summary() {
        println!("    · {line}");
    }
    println!(
        "  Up to {} MB of memory.",
        manifest.limits.memory_bytes / (1024 * 1024)
    );

    // And now actually run it — under **deny-all**, whatever it declared. What
    // is being checked is that it starts and draws, and a widget that needs a
    // grant to do either would be one that shows its user nothing until they
    // agree to something.
    let runtime = Runtime::new().map_err(|e| e.to_string())?;
    let compiled = runtime
        .compile(&bytes)
        .map_err(|e| format!("this is not a widget this host can load: {e}"))?;
    let mut instance = runtime
        .instantiate(
            &compiled,
            Arc::new(manifest),
            Policy::deny_all(),
            Services::none(),
        )
        .map_err(|e| format!("it would not start: {e}"))?;
    let view = instance
        .view()
        .map_err(|e| format!("it started and then would not draw: {e}"))?;

    let nodes = count(&view.root);
    println!("\n  It starts, and draws {nodes} nodes with nothing granted.");
    for line in instance.take_log() {
        println!("    {line}");
    }
    println!();
    Ok(())
}

fn count(node: &uniwidged_view::Node) -> usize {
    1 + node.children.iter().map(count).sum::<usize>()
}
