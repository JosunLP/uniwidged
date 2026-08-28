//! `uniwidged` — run the host, and everything around running it.
//!
//! ```text
//! uniwidged run [--config PATH]     start the desktop in the foreground
//! uniwidged new <name>              scaffold a widget that already builds
//! uniwidged check <widget.wasm>     is this a widget, and who signed it?
//! uniwidged install <widget.wasm>   install a widget, showing what it asks for
//! uniwidged enable|disable <name>   turn one on or off without forgetting it
//! uniwidged remove <name>           uninstall, and optionally forget its data
//! uniwidged keygen | sign           publish widgets under a key of your own
//! uniwidged trust|untrust|trusted   whose signatures this machine accepts
//! uniwidged list                    what is installed, and what it was granted
//! uniwidged lock <name>             pin a widget where it is
//! uniwidged unlock <name>           let it be dragged again
//! uniwidged move <name> [--x N …]   place a widget without a pointer
//! uniwidged configure <name>         the knobs a widget declared
//! uniwidged autostart enable        start with the session
//! uniwidged doctor                  what this session can and cannot do
//! uniwidged snapshot <name> [-o F]  render one widget headlessly to a PNG
//! ```
//!
//! `configure` is the other one worth explaining. A widget declares what it can
//! be configured with — a key, a type, a range, a default — and this is where a
//! value is checked against that declaration. Refusing here rather than later
//! is the whole point: a widget can then be written with no branch for a value
//! it cannot handle, because it will never be given one.
//!
//! `move` exists because a pointer is not always available: a widget can be
//! placed over SSH, from a dotfiles script, or back onto a monitor that was
//! unplugged while it was there.
//!
//! `doctor` is the one worth explaining. A desktop widget depends on properties
//! the platform may or may not provide — a compositing manager, a transparent
//! swapchain, a window manager that honours `_NET_WM_STATE_BELOW` — and when
//! one is missing the symptom is visual and the cause is not. So it reports
//! what the session actually offers, in the same words the start-up log uses.

mod autostart;
mod check;
mod configure;
mod new;
mod paths;
#[cfg(all(unix, not(target_os = "macos")))]
mod run;
mod trust;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let rest: Vec<String> = args.collect();
    let result = match command.as_deref() {
        Some("run") => command_run(&rest),
        Some("new") => new::create(&rest),
        Some("check") => check::check(&rest),
        Some("install") => command_install(&rest),
        Some("list") => command_list(&rest),
        Some("enable") => command_enable(&rest, true),
        Some("disable") => command_enable(&rest, false),
        Some("remove") => command_remove(&rest),
        Some("keygen") => trust::keygen(&rest),
        Some("sign") => trust::sign(&rest),
        Some("trust") => trust::trust(&rest),
        Some("untrust") => trust::untrust(&rest),
        Some("trusted") => trust::list(&rest),
        Some("lock") => command_lock(&rest, true),
        Some("unlock") => command_lock(&rest, false),
        Some("move") => command_move(&rest),
        Some("configure") => configure::command(&rest),
        Some("autostart") => autostart::command(&rest),
        Some("doctor") => command_doctor(),
        Some("snapshot") => command_snapshot(&rest),
        Some("--help" | "-h" | "help") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => Err(format!("`{other}` is not a command. Try `uniwidged help`.")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("uniwidged: {why}");
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    println!(
        "uniwidged — a cross-platform desktop widget framework\n\n\
         USAGE:\n  \
           uniwidged run [--config PATH]     start the desktop in the foreground\n  \
           uniwidged new <name> [--dir D] [--sdk P] [--capability C]…\n  \
           uniwidged check <widget.wasm>     is this a widget, and who signed it?\n  \
           uniwidged enable|disable <name>   turn one on or off\n  \
           uniwidged remove <name> [--purge] uninstall it\n  \
           uniwidged keygen [--out F]        a signing key of your own\n  \
           uniwidged sign <w.wasm> --key F   sign a widget you publish\n  \
           uniwidged trust <key> [--name N]  accept that key's signatures\n  \
           uniwidged untrust <key> | trusted\n  \
           uniwidged install <widget.wasm> [--allow-unsigned]\n  \
           uniwidged list                    what is installed, and what it was granted\n  \
           uniwidged lock <name>             pin a widget where it is\n  \
           uniwidged unlock <name>           let it be dragged again\n  \
           uniwidged move <name> [--x N] [--y N] [--width N] [--height N] [--output NAME]\n  \
           uniwidged configure <name> [--set k=v] [--reset k] [--reset-all]\n  \
           uniwidged autostart [enable|disable|status]\n  \
           uniwidged doctor                  what this session can and cannot do\n  \
           uniwidged snapshot <name> [-o F]  render one widget headlessly to a PNG\n"
    );
}

pub(crate) fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

pub(crate) fn config_path(args: &[String]) -> PathBuf {
    flag(args, "--config")
        .map(PathBuf::from)
        .unwrap_or_else(paths::config_file)
}

fn command_run(args: &[String]) -> Result<(), String> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        run::run(&config_path(args))
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = args;
        Err("this build has no shell for the current platform yet — \
             Windows is milestone M5 and macOS is M6, see ROADMAP.md"
            .into())
    }
}

fn command_install(args: &[String]) -> Result<(), String> {
    use uniwidged_host::config::{Config, WidgetConfig};
    use uniwidged_runtime::SecretStore;
    use uniwidged_runtime::manifest::Manifest;

    let component = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or("which widget? `uniwidged install <widget.wasm>`")?;
    let component = PathBuf::from(component)
        .canonicalize()
        .map_err(|e| format!("{component}: {e}"))?;
    let manifest_path = component.with_extension("toml");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest = Manifest::parse(&text).map_err(|e| e.to_string())?;

    // Provenance before consent. The list below is only worth reading if it is
    // the list the author wrote — a signature covering the component alone
    // would let anybody widen it, which is why `signature::digest` covers both.
    let allow_unsigned = args.iter().any(|a| a == "--allow-unsigned");
    let signed_by = match trust::provenance(&component, &manifest_path)? {
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

    // What the user is actually asked. Everything the runtime later enforces
    // is on this list, and nothing that is not.
    println!("\n  {} {}", manifest.name, manifest.version);
    match &signed_by {
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
        println!(
            "  Its keychain here is {}.\n",
            uniwidged_runtime::FileKeyring::new(paths::keyring_dir()).describe()
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

    let path = paths::config_file();
    let mut config = Config::read(&path).map_err(|e| e.to_string())?;
    let name = manifest.name.clone();
    config.widgets.retain(|w| w.name != name);
    config
        .widgets
        .push(WidgetConfig::granting_all(&name, component, &manifest));
    config.write(&path).map_err(|e| e.to_string())?;
    println!("  installed; recorded in {}", path.display());
    println!("  to decline one of those, remove it from `granted` there.\n");
    Ok(())
}

fn command_list(args: &[String]) -> Result<(), String> {
    use uniwidged_host::config::Config;

    let path = config_path(args);
    let config = Config::read(&path).map_err(|e| e.to_string())?;
    if config.widgets.is_empty() {
        println!("nothing installed ({})", path.display());
        return Ok(());
    }
    for widget in &config.widgets {
        let placement = &widget.placement;
        println!(
            "{}{}  {}x{} at {},{}{}",
            widget.name,
            if widget.enabled { "" } else { " (disabled)" },
            placement.width,
            placement.height,
            placement.x,
            placement.y,
            placement
                .output
                .as_ref()
                .map(|o| format!(" on {o}"))
                .unwrap_or_default(),
        );
        if placement.locked {
            println!("    locked in place");
        }
        if widget.granted.is_empty() {
            println!("    granted nothing");
        } else {
            println!("    granted {}", widget.granted.join(", "));
        }
    }
    Ok(())
}

/// Pins a widget where it is, or lets it be dragged again.
///
/// The switch people reach for once a widget is where they want it — and the
/// one they reach for again when they knock it out of place. A locked widget
/// still works: clicks reach it, it still updates. It just does not move.
fn command_lock(args: &[String], locked: bool) -> Result<(), String> {
    use uniwidged_host::config::Config;

    let name = args
        .first()
        .ok_or("which widget? `uniwidged lock <name>`")?;
    let path = config_path(args);
    let mut config = Config::read(&path).map_err(|e| e.to_string())?;
    let widget = config
        .widget_mut(name)
        .ok_or_else(|| format!("`{name}` is not installed; try `uniwidged list`"))?;
    widget.placement.locked = locked;
    config.write(&path).map_err(|e| e.to_string())?;
    println!(
        "  `{name}` is {}",
        if locked {
            "locked where it is"
        } else {
            "free to move again"
        }
    );
    Ok(())
}

/// Places a widget without a pointer.
///
/// Every field is optional and anything omitted is left alone, so `--y 900` is
/// a nudge rather than a reset. The coordinates are relative to the widget's
/// output when it has one — the same numbers `config.toml` holds, and the same
/// ones a drag writes back.
fn command_move(args: &[String]) -> Result<(), String> {
    use uniwidged_host::config::Config;

    let name = args
        .first()
        .ok_or("which widget? `uniwidged move <name> [--x N] [--y N]`")?;
    let path = config_path(args);
    let mut config = Config::read(&path).map_err(|e| e.to_string())?;
    let widget = config
        .widget_mut(name)
        .ok_or_else(|| format!("`{name}` is not installed; try `uniwidged list`"))?;

    let number = |name: &str| -> Result<Option<i64>, String> {
        match flag(args, name) {
            None => Ok(None),
            Some(value) => value
                .parse::<i64>()
                .map(Some)
                .map_err(|_| format!("`{value}` is not a number for {name}")),
        }
    };
    let mut changed = false;
    if let Some(x) = number("--x")? {
        widget.placement.x = x as i32;
        changed = true;
    }
    if let Some(y) = number("--y")? {
        widget.placement.y = y as i32;
        changed = true;
    }
    // A widget smaller than this cannot be grabbed to make it bigger again,
    // which is the same floor the pointer enforces — see
    // `uniwidged_host::interaction::MIN_SIZE`.
    let min = uniwidged_host::interaction::MIN_SIZE;
    if let Some(width) = number("--width")? {
        if width < min.0 as i64 {
            return Err(format!(
                "a widget may not be narrower than {} pixels",
                min.0
            ));
        }
        widget.placement.width = width as u32;
        changed = true;
    }
    if let Some(height) = number("--height")? {
        if height < min.1 as i64 {
            return Err(format!("a widget may not be shorter than {} pixels", min.1));
        }
        widget.placement.height = height as u32;
        changed = true;
    }
    if let Some(output) = flag(args, "--output") {
        // `--output ""` forgets the monitor, which is how a widget stored
        // against a screen that no longer exists is set free.
        widget.placement.output = (!output.is_empty()).then_some(output);
        changed = true;
    }
    if !changed {
        return Err(
            "nothing to change: give at least one of --x, --y, --width, --height, --output".into(),
        );
    }

    let placement = widget.placement.clone();
    config.write(&path).map_err(|e| e.to_string())?;
    println!(
        "  `{name}` is now {}x{} at {},{}{}",
        placement.width,
        placement.height,
        placement.x,
        placement.y,
        placement
            .output
            .as_deref()
            .map(|o| format!(" on {o}"))
            .unwrap_or_default(),
    );
    Ok(())
}

/// `uniwidged enable|disable <name>` — on or off, without forgetting it.
///
/// A disabled widget keeps its placement, its grants and its data. That is the
/// difference between this and `remove`, and it is why both exist: turning a
/// widget off to see whether it was the one making a noise should not cost the
/// half-hour somebody spent putting it where they wanted it.
fn command_enable(args: &[String], enabled: bool) -> Result<(), String> {
    use uniwidged_host::config::Config;

    let name = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or("which widget? `uniwidged enable <name>`")?;
    let path = config_path(args);
    let mut config = Config::read(&path).map_err(|e| e.to_string())?;
    let widget = config
        .widget_mut(name)
        .ok_or_else(|| format!("`{name}` is not installed; try `uniwidged list`"))?;

    if widget.enabled == enabled {
        println!(
            "  `{name}` is already {}",
            if enabled { "enabled" } else { "disabled" }
        );
        return Ok(());
    }
    widget.enabled = enabled;
    config.write(&path).map_err(|e| e.to_string())?;
    println!(
        "  `{name}` is {}; it takes effect the next time the desktop starts",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

/// `uniwidged remove <name> [--purge]` — uninstall.
///
/// The widget's **data is kept** unless `--purge` says otherwise. Removing a
/// widget is something people do to try one out and change their minds, and a
/// note-taking widget that silently took the notes with it would be a bug
/// nobody could undo. `--purge` says what it does before doing it.
fn command_remove(args: &[String]) -> Result<(), String> {
    use uniwidged_host::config::Config;

    let name = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or("which widget? `uniwidged remove <name> [--purge]`")?;
    let path = config_path(args);
    let mut config = Config::read(&path).map_err(|e| e.to_string())?;
    if config.widget(name).is_none() {
        return Err(format!("`{name}` is not installed; try `uniwidged list`"));
    }
    config.widgets.retain(|widget| widget.name != *name);
    config.write(&path).map_err(|e| e.to_string())?;
    println!("  removed `{name}` from {}", path.display());

    let data = paths::widget_data_dir(name);
    if args.iter().any(|a| a == "--purge") {
        match std::fs::remove_dir_all(&data) {
            Ok(()) => println!("  and deleted {}", data.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                println!("  it had stored nothing")
            }
            Err(e) => eprintln!("uniwidged: could not delete {}: {e}", data.display()),
        }
    } else if data.exists() {
        // Said rather than left to be discovered. Somebody removing a widget to
        // reinstall it wants this; somebody removing it for good wants to know
        // it is still there.
        println!(
            "  its data is still in {} — `--purge` deletes it",
            data.display()
        );
    }
    Ok(())
}

fn command_doctor() -> Result<(), String> {
    println!("uniwidged doctor\n");
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        run::doctor();
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    println!("  no shell for this platform yet — see ROADMAP.md");

    // The renderer is platform-independent, so it can always be asked.
    match uniwidged_render::Renderer::new() {
        Ok(renderer) => {
            let info = renderer.adapter_info();
            println!("\n  renderer: {} ({:?})", info.name, info.backend);
        }
        Err(e) => println!("\n  renderer: unavailable — {e}"),
    }
    Ok(())
}

fn command_snapshot(args: &[String]) -> Result<(), String> {
    use uniwidged_compose::Theme;
    use uniwidged_core::Size;
    use uniwidged_host::Host;
    use uniwidged_host::config::Config;
    use uniwidged_runtime::manifest::Manifest;

    let name = args
        .first()
        .ok_or("which widget? `uniwidged snapshot <name>`")?;
    let out = flag(args, "-o").unwrap_or_else(|| format!("{name}.png"));

    let path = paths::config_file();
    let config = Config::read(&path).map_err(|e| e.to_string())?;
    let widget = config
        .widget(name)
        .ok_or_else(|| format!("`{name}` is not installed; try `uniwidged list`"))?;

    let wasm = std::fs::read(&widget.component)
        .map_err(|e| format!("{}: {e}", widget.component.display()))?;
    let manifest_text = std::fs::read_to_string(widget.manifest_path())
        .map_err(|e| format!("{}: {e}", widget.manifest_path().display()))?;
    let manifest = Manifest::parse(&manifest_text).map_err(|e| e.to_string())?;
    let policy = widget.policy(&manifest).map_err(|e| e.to_string())?;

    let mut host = Host::new(Theme::default(), std::time::Duration::from_millis(16))
        .map_err(|e| e.to_string())?;
    let id = host
        .install(&wasm, manifest, policy)
        .map_err(|e| e.to_string())?;
    let size = Size::new(
        widget.placement.width as f64,
        widget.placement.height as f64,
    );
    let surface = host.attach_readback(id, size).map_err(|e| e.to_string())?;
    host.tick(std::time::Instant::now(), std::time::SystemTime::now())
        .map_err(|e| e.to_string())?;
    host.renderer_mut()
        .save_png(surface, std::path::Path::new(&out))
        .map_err(|e| e.to_string())?;
    println!("wrote {out}");
    Ok(())
}
