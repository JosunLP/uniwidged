//! Installing a widget, wherever it came from.
//!
//! One path, used by `uniwidged install <file>` and by `uniwidged add <name>`
//! after a download. That is the point of the file: a widget fetched from a
//! registry must go through the *same* provenance check, the same consent
//! screen and the same configuration write as one somebody sent by email.
//! Downloading is not trusting, and a second install path is how a project
//! ends up with two answers to "was this signed by somebody you trust".
//!
//! ## What is preserved when a widget is installed over itself
//!
//! Where it sits, what it was configured with, and whether it was switched
//! off. An installation is a new *version of a widget*, not a new widget, and
//! somebody who has spent five minutes putting a widget where they want it
//! should not have it jump back to the corner because its author shipped a
//! bug fix.
//!
//! What is **not** preserved is the grant, on an explicit `install`: the list
//! is printed and agreed to again, because it may have changed and the whole
//! design says a user approved *that manifest's* list. An `update` is the
//! other way round — see [`Grant::AsBefore`], where the new manifest is
//! required to ask for nothing the old grant did not already cover.

use std::path::{Path, PathBuf};

use uniwidged_host::config::{Config, WidgetConfig, capability_text};
use uniwidged_runtime::manifest::Manifest;

use crate::trust;

/// What the widget being installed should be granted.
pub enum Grant {
    /// Everything its manifest declares, having shown the user the list.
    /// What an explicit installation does.
    Everything,
    /// Exactly what the previous version was granted, and refuse if the new
    /// manifest wants anything more.
    ///
    /// An unattended upgrade may not widen a permission. A version that asks
    /// for more is not refused *forever* — it is refused *silently*, and the
    /// user is told to run `uniwidged add`, where the new list is printed and
    /// agreed to like any other.
    AsBefore(Vec<String>),
}

/// `uniwidged install <widget.wasm> [--allow-unsigned]`
pub fn command(args: &[String]) -> Result<(), String> {
    let component = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or("which widget? `uniwidged install <widget.wasm>`")?;
    let component = PathBuf::from(component)
        .canonicalize()
        .map_err(|e| format!("{component}: {e}"))?;
    let manifest_path = component.with_extension("toml");
    let allow_unsigned = args.iter().any(|a| a == "--allow-unsigned");
    install(
        &component,
        &manifest_path,
        allow_unsigned,
        Grant::Everything,
    )?;
    Ok(())
}

/// Installs a widget that is already on disk, and reports what was recorded.
///
/// The component is recorded by the path given, so a widget installed from a
/// download directory that is later cleared stops working. `uniwidged add`
/// copies into the widget store first for exactly that reason.
pub fn install(
    component: &Path,
    manifest_path: &Path,
    allow_unsigned: bool,
    grant: Grant,
) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest = Manifest::parse(&text).map_err(|e| e.to_string())?;

    // Provenance before consent. The list below is only worth reading if it is
    // the list the author wrote — a signature covering the component alone
    // would let anybody widen it, which is why `signature::digest` covers both.
    let signed_by = match trust::provenance(component, manifest_path)? {
        trust::Provenance::Trusted(who) => Some(who),
        trust::Provenance::Refused(why) => {
            // Never installable, `--allow-unsigned` or not: a signature that
            // does not verify is a positive claim that something is wrong,
            // where a missing one is only an absence.
            return Err(format!(
                "{} has a signature and it does not check out: {why}",
                component.display()
            ));
        }
        trust::Provenance::Unsigned if allow_unsigned => None,
        trust::Provenance::Unsigned => {
            return Err(format!(
                "{} is not signed.\n  \
                 If you built it yourself, `uniwidged install --allow-unsigned` \
                 says so out loud.\n  \
                 If somebody sent it to you, ask them for the signature and \
                 their key, then `uniwidged trust <key>`.",
                component.display()
            ));
        }
    };

    let declared: Vec<String> = manifest.capabilities.iter().map(capability_text).collect();
    let granted = match &grant {
        Grant::Everything => declared.clone(),
        Grant::AsBefore(before) => {
            // The check that makes an unattended upgrade safe. Anything the
            // new manifest asks for and the old grant does not cover is a
            // permission nobody agreed to.
            let new: Vec<&String> = declared.iter().filter(|c| !before.contains(c)).collect();
            if !new.is_empty() {
                return Err(format!(
                    "this version of `{}` asks for something the installed one \
                     did not: {}.\n  That is a decision rather than an update, so \
                     `uniwidged add {}` will show you the whole list and ask.",
                    manifest.name,
                    new.iter()
                        .map(|c| c.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    manifest.name
                ));
            }
            // Intersected, not carried over whole: a capability the old
            // version had and the new one dropped must not stay granted, or
            // the configuration would grant something no manifest declares.
            before
                .iter()
                .filter(|c| declared.contains(c))
                .cloned()
                .collect()
        }
    };

    if matches!(grant, Grant::Everything) {
        describe(&manifest, signed_by.as_deref());
    }

    let path = crate::paths::config_file();
    let mut config = Config::read(&path).map_err(|e| e.to_string())?;
    let name = manifest.name.clone();

    // Where it sat, what it was set to, and whether it was switched off. See
    // the module note: a new version of a widget is not a new widget.
    let previous = config.widget(&name).cloned();
    config.widgets.retain(|w| w.name != name);
    let mut entry = WidgetConfig::granting_all(&name, component.to_path_buf(), &manifest);
    entry.granted = granted;
    if let Some(previous) = previous {
        entry.placement = previous.placement;
        entry.settings = previous.settings;
        entry.enabled = previous.enabled;
    }
    config.widgets.push(entry);
    config.write(&path).map_err(|e| e.to_string())?;
    crate::nudge_desktop();
    println!("  installed; recorded in {}", path.display());
    if matches!(grant, Grant::Everything) {
        println!("  to decline one of those, remove it from `granted` there.\n");
    }
    Ok(manifest)
}

/// The consent screen: everything the runtime will enforce, and nothing that
/// is not on the manifest.
pub fn describe(manifest: &Manifest, signed_by: Option<&str>) {
    println!("\n  {} {}", manifest.name, manifest.version);
    match signed_by {
        Some(who) => println!("  signed by {who}"),
        None => println!(
            "  NOT SIGNED — you are trusting this because you chose to, and \
             nothing here checked anything"
        ),
    }
    if !manifest.description.is_empty() {
        println!("  {}", manifest.description);
    }
    println!("\n  This widget wants to:");
    for line in manifest.consent_summary() {
        println!("    · {line}");
    }
    println!(
        "  It may use up to {} MB of memory.\n",
        manifest.limits.memory_bytes / (1024 * 1024)
    );
    // "store passwords in your keychain" is a sentence whose weight depends
    // entirely on what the keychain is, and a user cannot be asked to consent
    // to something they have not been told. So the one capability whose
    // backing store is a judgement call names it here, at the moment of the
    // decision, rather than only in `doctor` where nobody looks first.
    if manifest
        .capabilities
        .iter()
        .any(|c| matches!(c, uniwidged_runtime::Capability::Secrets))
    {
        use uniwidged_runtime::SecretStore;
        println!(
            "  Its keychain here is {}.\n",
            uniwidged_runtime::FileKeyring::new(crate::paths::keyring_dir()).describe()
        );
    }

    if !manifest.settings.is_empty() {
        // Said at installation because it is the moment somebody is looking:
        // a widget whose knobs are only discoverable by running another
        // command is a widget whose knobs nobody finds.
        println!(
            "  It can be configured: {}",
            manifest
                .settings
                .iter()
                .map(|setting| setting.key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!("  `uniwidged configure {}`\n", manifest.name);
    }
}
