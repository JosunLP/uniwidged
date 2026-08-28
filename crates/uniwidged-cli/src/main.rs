//! `uniwidged` — run the host, and everything around running it.
//!
//! ```text
//! uniwidged run [--config PATH]     start the desktop in the foreground
//! uniwidged new <name>              scaffold a widget that already builds
//! uniwidged install <widget.wasm>   install a widget, showing what it asks for
//! uniwidged list                    what is installed, and what it was granted
//! uniwidged lock <name>             pin a widget where it is
//! uniwidged unlock <name>           let it be dragged again
//! uniwidged move <name> [--x N …]   place a widget without a pointer
//! uniwidged doctor                  what this session can and cannot do
//! uniwidged snapshot <name> [-o F]  render one widget headlessly to a PNG
//! ```
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

mod new;
mod paths;
#[cfg(all(unix, not(target_os = "macos")))]
mod run;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let rest: Vec<String> = args.collect();
    let result = match command.as_deref() {
        Some("run") => command_run(&rest),
        Some("new") => new::create(&rest),
        Some("install") => command_install(&rest),
        Some("list") => command_list(&rest),
        Some("lock") => command_lock(&rest, true),
        Some("unlock") => command_lock(&rest, false),
        Some("move") => command_move(&rest),
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
           uniwidged install <widget.wasm>   install a widget, showing what it asks for\n  \
           uniwidged list                    what is installed, and what it was granted\n  \
           uniwidged lock <name>             pin a widget where it is\n  \
           uniwidged unlock <name>           let it be dragged again\n  \
           uniwidged move <name> [--x N] [--y N] [--width N] [--height N] [--output NAME]\n  \
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

fn config_path(args: &[String]) -> PathBuf {
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
    use uniwidged_runtime::manifest::Manifest;

    let component = args
        .first()
        .ok_or("which widget? `uniwidged install <widget.wasm>`")?;
    let component = PathBuf::from(component)
        .canonicalize()
        .map_err(|e| format!("{component}: {e}"))?;
    let manifest_path = component.with_extension("toml");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest = Manifest::parse(&text).map_err(|e| e.to_string())?;

    // What the user is actually asked. Everything the runtime later enforces
    // is on this list, and nothing that is not.
    println!("\n  {} {}", manifest.name, manifest.version);
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
