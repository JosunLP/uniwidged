//! `uniwidged` — run the host, and everything around running it.
//!
//! ```text
//! uniwidged run [--config PATH]     start the desktop in the foreground
//! uniwidged new <name>              scaffold a widget that already builds
//! uniwidged check <widget.wasm>     is this a widget, and who signed it?
//! uniwidged install <widget.wasm>   install a widget, showing what it asks for
//! uniwidged registry add <url>      read a registry, pinned to its key
//! uniwidged search [term]           what the registries offer
//! uniwidged add <name>[@version]    install one from a registry
//! uniwidged update [name]           what is newer, and take it
//! uniwidged enable|disable <name>   turn one on or off without forgetting it
//! uniwidged remove <name>           uninstall, and optionally forget its data
//! uniwidged keygen | sign           publish widgets under a key of your own
//! uniwidged trust|untrust|trusted   whose signatures this machine accepts
//! uniwidged list                    what is installed, and what it was granted
//! uniwidged lock <name>             pin a widget where it is
//! uniwidged unlock <name>           let it be dragged again
//! uniwidged move <name> [--x N …]   place a widget without a pointer
//! uniwidged configure <name>         the knobs a widget declared
//! uniwidged reload                  tell a running desktop to read it again
//! uniwidged status                  what is on the screen right now
//! uniwidged log [-n N]              what the running desktop has been saying
//! uniwidged autostart enable        start with the session
//! uniwidged doctor                  what this session can and cannot do
//! uniwidged snapshot <name> [-o F]  render one widget headlessly to a PNG
//! uniwidged completions <shell>     a completion script for bash, zsh or fish
//! ```
//!
//! `reload` is worth a sentence too. A desktop reads its configuration at
//! start-up, and every command here that writes one tells a running desktop
//! about it — so `reload` is only needed after editing the file by hand. What
//! it sends is `SIGHUP`, which has meant *look at your configuration again* to
//! a daemon for forty years; see `daemon`, where the dangerous part turns out
//! to be the pid rather than the signal.
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
mod completions;
mod configure;
mod daemon;
mod install;
mod log;
mod new;
mod paths;
mod registry;
mod run;
mod state;
mod trust;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let rest: Vec<String> = args.collect();
    // `<command> --help` anywhere on the line is a question about the command,
    // never an argument to it: no command here takes a value spelled that way.
    // And an option the command does not know stops it before it does
    // anything — `remove clock --purgee` must not remove the clock.
    let known = command
        .as_deref()
        .and_then(|name| completions::COMMANDS.iter().find(|c| c.name == name));
    let result = match (command.as_deref(), known.map(|c| read_options(c, &rest))) {
        (Some(name), Some(Ok(true))) => command_help(name),
        (_, Some(Err(unknown))) => Err(unknown),
        _ => run_command(command.as_deref(), &rest),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("uniwidged: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_command(command: Option<&str>, rest: &[String]) -> Result<(), String> {
    let rest = rest.to_vec();
    match command {
        Some("help") if !rest.is_empty() => command_help(&rest[0]),
        Some("run") => command_run(&rest),
        Some("new") => new::create(&rest),
        Some("check") => check::check(&rest),
        Some("install") => install::command(&rest),
        Some("registry") => registry::command(&rest),
        Some("search") => registry::search(&rest),
        Some("add") => registry::add(&rest),
        Some("update") => registry::update(&rest),
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
        Some("reload") => command_reload(&rest),
        Some("log") => log::command(&rest),
        Some("status") => command_status(&rest),
        Some("autostart") => autostart::command(&rest),
        Some("doctor") => command_doctor(),
        Some("snapshot") => command_snapshot(&rest),
        Some("completions") => completions::command(&rest),
        Some("--version" | "-V" | "version") => {
            // The ABI version too, because "which uniwidged is this" is nearly
            // always asked about a widget that will not load, and the answer is
            // the number in its manifest against the number here.
            let (oldest, newest) = (
                uniwidged_runtime::ABI_OLDEST,
                uniwidged_runtime::ABI_VERSION,
            );
            if oldest == newest {
                println!(
                    "uniwidged {} (widget ABI {newest})",
                    env!("CARGO_PKG_VERSION")
                );
            } else {
                println!(
                    "uniwidged {} (widget ABI {newest}, runs {oldest} to {newest})",
                    env!("CARGO_PKG_VERSION")
                );
            }
            Ok(())
        }
        Some("--help" | "-h" | "help") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => Err(format!("`{other}` is not a command. Try `uniwidged help`.")),
    }
}

/// The whole command line, one usage per line. `help <command>` reads its
/// lines out of this, so a command's usage is written once.
const USAGE: &str = "uniwidged — a cross-platform desktop widget framework\n\n\
         USAGE:\n  \
           uniwidged run [--config PATH]     start the desktop in the foreground\n  \
           uniwidged new <name> [--dir D] [--sdk P] [--capability C]…\n  \
           uniwidged check <widget.wasm> [--manifest F]  is this a widget, and who signed it?\n  \
           uniwidged enable|disable <name>   turn one on or off\n  \
           uniwidged remove <name> [--purge] uninstall it\n  \
           uniwidged keygen [--out F]        a signing key of your own\n  \
           uniwidged sign <w.wasm> --key F [--manifest F]  sign a widget you publish\n  \
           uniwidged trust <key> [--name N]  accept that key's signatures\n  \
           uniwidged untrust <key>           stop accepting that key\n  \
           uniwidged trusted                 the keys this machine accepts\n  \
           uniwidged install <widget.wasm> [--manifest F] [--allow-unsigned]\n  \
           uniwidged registry add <url> --key <key> [--name N]\n  \
           uniwidged registry remove <name> | list\n  \
           uniwidged registry publish --key F --base-url U <w.wasm>…\n  \
           uniwidged search [term] [--refresh] [--offline]\n  \
           uniwidged add <name>[@version] [--registry N]\n  \
           uniwidged update [name] [--check]   what is newer, and take it\n  \
           uniwidged list [--names]          what is installed, and what it was granted\n  \
           uniwidged lock <name>             pin a widget where it is\n  \
           uniwidged unlock <name>           let it be dragged again\n  \
           uniwidged move <name> [--x N] [--y N] [--width N] [--height N] [--output NAME]\n  \
           uniwidged configure <name> [--set k=v] [--reset k] [--reset-all]\n  \
           uniwidged reload                  tell a running desktop to read it again\n  \
           uniwidged status                  what is on the screen right now\n  \
           uniwidged log [-n N] [-f]         what the running desktop has been saying\n  \
           uniwidged autostart [enable|disable|status]\n  \
           uniwidged doctor                  what this session can and cannot do\n  \
           uniwidged snapshot <name> [-o F]  render one widget headlessly to a PNG\n  \
           uniwidged completions <shell>     a completion script for bash, zsh or fish\n  \
           uniwidged help [command]          every command, or more about one\n  \
           uniwidged --version\n";

fn print_usage() {
    println!("{USAGE}");
    println!("  `uniwidged help <command>` says more about one of them.\n");
}

/// Reads `args` against the options `command` knows: `Ok(true)` when it asks
/// for help, `Err` naming the first option it does not know.
///
/// Only a known option that takes a value consumes the token after it, so
/// `--config -h` is a file called `-h` while `--purgee --help` is a typo and
/// then a question. Help wins over an unknown option: somebody asking what
/// the options are is told, rather than told off.
fn read_options(command: &completions::Command, args: &[String]) -> Result<bool, String> {
    const SWITCHES: &[&str] = &[
        "--allow-unsigned",
        "--purge",
        "--names",
        "--refresh",
        "--offline",
        "--check",
        "--reset-all",
        "-f",
        "--follow",
        "--background",
    ];
    let mut unknown = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            return Ok(true);
        }
        if !arg.starts_with('-') || arg == "-" {
            continue;
        }
        let known = arg == "--config" || command.flags.contains(&arg.as_str());
        if !known {
            unknown.get_or_insert_with(|| arg.clone());
        } else if !SWITCHES.contains(&arg.as_str()) && args.next().is_none() {
            // `remove clock --config` must not fall back to the default
            // configuration and remove the clock from there.
            return Err(format!("`{arg}` needs a value"));
        }
    }
    match unknown {
        Some(option) => Err(format!(
            "`{option}` is not an option of `{name}`. `uniwidged help {name}` lists them.",
            name = command.name
        )),
        None => Ok(false),
    }
}

/// `uniwidged help <command>`, and `uniwidged <command> --help`.
///
/// The summary and flags come from the completion table and the usage from
/// [`USAGE`], so nothing here is a third copy of either that could drift.
fn command_help(name: &str) -> Result<(), String> {
    let command = completions::COMMANDS
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| format!("`{name}` is not a command. Try `uniwidged help`."))?;
    println!("uniwidged {} — {}\n", command.name, command.summary);
    let usage: Vec<&str> = USAGE
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.strip_prefix("uniwidged ")
                .and_then(|rest| rest.split_whitespace().next())
                .is_some_and(|word| word.split('|').any(|w| w == name))
        })
        .collect();
    if !usage.is_empty() {
        println!("USAGE:");
        for line in usage {
            println!("  {line}");
        }
        println!();
    }
    if !command.flags.is_empty() {
        println!("FLAGS:\n  {}  --config PATH\n", command.flags.join("  "));
    }
    println!(
        "More in `man uniwidged`, and at\n  https://josunlp.github.io/uniwidged/reference/cli#{}",
        command.name
    );
    Ok(())
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
    // Started by the session rather than by a person: on Windows the binary is
    // a console program, and a console window open on the desktop for as long
    // as the widgets are is not something anybody asked for. Everything a
    // desktop says still reaches the log. See `autostart`.
    #[cfg(windows)]
    if args.iter().any(|a| a == "--background") {
        // SAFETY: detaches this process from its console, if it has one.
        unsafe { windows_sys::Win32::System::Console::FreeConsole() };
    }
    run::run(&config_path(args))
}

fn command_reload(args: &[String]) -> Result<(), String> {
    daemon::command(args)
}

/// Tells a running desktop that the configuration it read has changed.
///
/// Called by every command that writes one. Quiet when there is no desktop:
/// most of the time there is not, and a line every time would be noise.
pub(crate) fn nudge_desktop() {
    daemon::nudge_quietly();
}

fn command_list(args: &[String]) -> Result<(), String> {
    use uniwidged_host::config::Config;

    let path = config_path(args);
    let config = Config::read(&path).map_err(|e| e.to_string())?;
    if args.iter().any(|a| a == "--names") {
        // One name per line and nothing else — what a shell completion or a
        // script wants, and the one output of this command that is a format
        // rather than a sentence. Nothing installed is nothing printed.
        for widget in &config.widgets {
            println!("{}", widget.name);
        }
        return Ok(());
    }
    if config.widgets.is_empty() {
        println!("nothing installed ({})", path.display());
        return Ok(());
    }
    // What is *installed* and what is *on the screen* are different questions,
    // and a widget that is enabled here and missing there is exactly the case
    // somebody running this is trying to explain. `None` when no desktop is
    // running, which is most of the time and is not worth a word.
    let running = state::current().ok().flatten();
    for widget in &config.widgets {
        let placement = &widget.placement;
        println!(
            "{}{}{}  {}x{} at {},{}{}",
            widget.name,
            if widget.enabled { "" } else { " (disabled)" },
            live(running.as_ref(), &widget.name, widget.enabled),
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

/// How a widget in `list` is annotated with what the running desktop is doing.
///
/// Empty when there is no desktop, because then every line would carry the same
/// noise. `not running` only for a widget the file says is *enabled*: a
/// disabled one is already labelled, and saying both would be saying the same
/// thing twice.
fn live(running: Option<&state::State>, name: &str, enabled: bool) -> String {
    let Some(state) = running else {
        return String::new();
    };
    match state.widgets.iter().find(|widget| widget.name == name) {
        Some(widget) if widget.health == state::Health::Running => " (on screen)".into(),
        Some(widget) => format!(" ({})", widget.health),
        // Disabled is why it is not running, and the line already says so.
        None if enabled => " (not running)".into(),
        None => String::new(),
    }
}

/// `uniwidged status` — what is on the screen right now.
///
/// The question `list` cannot answer, because `list` reads a file that says
/// what somebody asked for. This one reads what a running desktop wrote about
/// what it managed to do — which widget is drawing, which one it has given up
/// on, where each of them actually is, and what kind of surface this session
/// gave it.
fn command_status(_args: &[String]) -> Result<(), String> {
    let state = match state::current()? {
        Some(state) => state,
        None => {
            println!("  no desktop is running. `uniwidged run` starts one.");
            return Ok(());
        }
    };
    let locale = uniwidged_runtime::services::Locale::system();
    println!(
        "  desktop {} since {} {}",
        state.pid,
        locale.format_date(state.since),
        locale.format_time(state.since),
    );
    if !state.shell.is_empty() {
        println!("  shell: {}", state.shell);
    }
    if state.widgets.is_empty() {
        println!("  nothing on the screen");
    }
    for widget in &state.widgets {
        println!("\n  {} {}  {}", widget.name, widget.version, widget.health);
        println!(
            "    {}x{} at {},{}{}{}",
            widget.width,
            widget.height,
            widget.x,
            widget.y,
            widget
                .output
                .as_deref()
                .map(|o| format!(" on {o}"))
                .unwrap_or_default(),
            if widget.locked { ", locked" } else { "" },
        );
    }
    println!("\n  what it has been saying: `uniwidged log`");
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
    nudge_desktop();
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
    nudge_desktop();
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
    nudge_desktop();
    println!(
        "  `{name}` is {}",
        if enabled { "enabled" } else { "disabled" }
    );
    // A running desktop starts or stops it on the nudge above. It is the one
    // thing a reload used to refuse — a widget arriving or leaving is a window
    // created or destroyed, and doing half of that on a signal is how a desktop
    // ends up with a window nothing draws into. `uniwidged_cli::run` does the
    // whole of each half or none of it, and this line is no longer a promise
    // that something happens later.
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
    // Kept before the entry goes, so that `--purge` can find the files a
    // download left in the widget store.
    let component = config.widget(name).map(|widget| widget.component.clone());
    config.widgets.retain(|widget| widget.name != *name);
    config.write(&path).map_err(|e| e.to_string())?;
    nudge_desktop();
    println!("  removed `{name}` from {}", path.display());

    let data = paths::widget_data_dir(name);
    if args.iter().any(|a| a == "--purge") {
        // A widget that came from a registry has a copy in the store, and
        // somebody purging a widget means the widget rather than only what it
        // wrote. One that was installed from a file somewhere else is left
        // exactly where its owner put it — deleting a file this program did
        // not put there is not this command's business.
        if let Some(component) = component
            && component.starts_with(paths::widget_store_dir())
        {
            for file in [
                component.clone(),
                component.with_extension("toml"),
                trust::signature_path(&component),
            ] {
                match std::fs::remove_file(&file) {
                    Ok(()) => println!("  and deleted {}", file.display()),
                    // A widget with no signature has no `.sig`, which is not a
                    // failure worth a line.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => eprintln!("uniwidged: could not delete {}: {e}", file.display()),
                }
            }
        }
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
    run::doctor();

    // The renderer is platform-independent, so it can always be asked.
    match uniwidged_render::Renderer::new() {
        Ok(renderer) => {
            let info = renderer.adapter_info();
            println!("\n  renderer: {} ({:?})", info.name, info.backend);
        }
        Err(e) => println!("\n  renderer: unavailable — {e}"),
    }

    // Where this machine would fetch a widget from, and whose signatures it
    // would accept. Both are answers to "why can I not install this", which is
    // the question `doctor` is usually being asked.
    match registry::read_registries() {
        Ok(registries) if registries.is_empty() => {
            println!("\n  registries: none — `uniwidged registry add <url> --key <key>`")
        }
        Ok(registries) => {
            println!("\n  registries:");
            for registry in registries {
                println!("    {}  {}", registry.name, registry.url);
            }
        }
        Err(why) => println!("\n  registries: unreadable — {why}"),
    }
    match trust::trusted() {
        Ok(keys) if keys.is_empty() => {
            println!("  trusted keys: none, so every widget needs --allow-unsigned")
        }
        Ok(keys) => println!("  trusted keys: {}", keys.keys().len()),
        Err(why) => println!("  trusted keys: unreadable — {why}"),
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

    let path = config_path(args);
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
