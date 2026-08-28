//! `uniwidged configure` — the knobs a widget declared.
//!
//! ```text
//! uniwidged configure <name>                  what it takes, and what it is set to
//! uniwidged configure <name> --set k=v …      change one or more
//! uniwidged configure <name> --reset k …      back to the widget's default
//! uniwidged configure <name> --reset-all
//! ```
//!
//! ## Everything is checked here, not later
//!
//! The value is parsed against the *declared* type and then checked against
//! the declared range or option list, so `--set warn-at=banana` fails at the
//! moment somebody types it. That is the whole reason settings are declared
//! rather than being a string map: a widget can be written with no branch for
//! a value it cannot handle, because it will never be given one.
//!
//! ## What is written
//!
//! Only what differs from the widget's own default. Writing out a value equal
//! to the default would pin this version's default into the file for good, so
//! a widget that later changed its mind about a sensible starting point would
//! never reach the people who never touched it.

use std::collections::BTreeMap;

use uniwidged_host::config::Config;
use uniwidged_runtime::manifest::Manifest;
use uniwidged_runtime::settings::SettingValue;

pub fn command(args: &[String]) -> Result<(), String> {
    let name = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or("which widget? `uniwidged configure <name>`")?;
    let path = crate::config_path(args);
    let mut config = Config::read(&path).map_err(|e| e.to_string())?;
    let widget = config
        .widget(name)
        .ok_or_else(|| format!("`{name}` is not installed; try `uniwidged list`"))?;

    let manifest_text = std::fs::read_to_string(widget.manifest_path())
        .map_err(|e| format!("{}: {e}", widget.manifest_path().display()))?;
    let manifest = Manifest::parse(&manifest_text).map_err(|e| e.to_string())?;

    let sets = flags(args, "--set");
    let resets = flags(args, "--reset");
    let reset_all = args.iter().any(|a| a == "--reset-all");

    if sets.is_empty() && resets.is_empty() && !reset_all {
        return show(&manifest, widget);
    }
    if manifest.settings.is_empty() {
        return Err(format!(
            "`{name}` declares no settings, so there is nothing to change"
        ));
    }

    // Start from what the widget would actually run with, so `--set` on one
    // key leaves the rest exactly as they were rather than resetting them.
    let (mut values, _) = widget.settings(&manifest);

    if reset_all {
        values = manifest
            .settings
            .iter()
            .map(|setting| (setting.key.clone(), setting.default.clone()))
            .collect();
    }
    for key in &resets {
        let setting = declared(&manifest, key)?;
        values.insert(setting.key.clone(), setting.default.clone());
    }
    for assignment in &sets {
        // `--set k=v`, split on the *first* `=`, because a text value may
        // perfectly well contain one and a key may not.
        let (key, text) = assignment.split_once('=').ok_or_else(|| {
            format!("`{assignment}` is not `key=value` — try `--set warn-at=0.9`")
        })?;
        let setting = declared(&manifest, key)?;
        let value = setting.parse(text).map_err(|e| e.to_string())?;
        values.insert(setting.key.clone(), value);
    }

    let widget = config.widget_mut(name).expect("found above");
    widget.set_settings(&manifest, &values);
    config.write(&path).map_err(|e| e.to_string())?;

    println!("  `{name}` is now:");
    for setting in &manifest.settings {
        print_one(setting, &values);
    }
    // Said, because it is not obvious and the alternative is somebody changing
    // a value, seeing nothing happen, and changing it again.
    println!("\n  a running desktop picks this up the next time it starts");
    Ok(())
}

/// Prints what the widget takes and what it is set to.
///
/// The declared type is shown beside every value — the range, the options, the
/// word `true or false` — because a form somebody has to guess the shape of is
/// a form that gets guessed wrong.
fn show(manifest: &Manifest, widget: &uniwidged_host::config::WidgetConfig) -> Result<(), String> {
    if manifest.settings.is_empty() {
        println!("  `{}` declares no settings", manifest.name);
        return Ok(());
    }
    let (values, complaints) = widget.settings(manifest);
    println!("\n  {} {}\n", manifest.name, manifest.version);
    for setting in &manifest.settings {
        print_one(setting, &values);
    }
    if !complaints.is_empty() {
        // A line in the file that is being ignored has to be findable.
        // Silently using the default is how a typo becomes a week of
        // wondering why a setting does nothing.
        println!("\n  and in {}:", widget.name);
        for complaint in complaints {
            println!("    ! {complaint}");
        }
    }
    println!(
        "\n  `uniwidged configure {} --set key=value`\n",
        manifest.name
    );
    Ok(())
}

fn print_one(
    setting: &uniwidged_runtime::settings::Setting,
    values: &BTreeMap<String, SettingValue>,
) {
    let value = values
        .get(&setting.key)
        .map(SettingValue::display)
        .unwrap_or_default();
    let default = if values.get(&setting.key) == Some(&setting.default) {
        " (default)"
    } else {
        ""
    };
    println!("    {} = {value}{default}", setting.key);
    println!("      {} — {}", setting.label, setting.describe());
}

fn declared<'m>(
    manifest: &'m Manifest,
    key: &str,
) -> Result<&'m uniwidged_runtime::settings::Setting, String> {
    manifest
        .settings
        .iter()
        .find(|setting| setting.key == key)
        .ok_or_else(|| {
            let known: Vec<&str> = manifest
                .settings
                .iter()
                .map(|setting| setting.key.as_str())
                .collect();
            if known.is_empty() {
                format!("`{}` declares no settings", manifest.name)
            } else {
                // The list, because a typo is the likeliest reason to be here
                // and the fix is one character away.
                format!(
                    "`{}` does not declare `{key}`; it declares {}",
                    manifest.name,
                    known.join(", ")
                )
            }
        })
}

/// Every value given for a repeatable flag.
///
/// Repeatable because changing two settings should be one command and one
/// write, not two — a form is saved all at once, and a command line that
/// cannot express that makes somebody run it twice.
fn flags(args: &[String], name: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, arg)| arg.as_str() == name)
        .filter_map(|(i, _)| args.get(i + 1))
        .filter(|value| !value.starts_with("--"))
        .cloned()
        .collect()
}
