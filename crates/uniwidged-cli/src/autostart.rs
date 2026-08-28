//! Starting with the session.
//!
//! A desktop widget that has to be launched by hand is a desktop widget
//! somebody stops using in a week. This is the piece M7 named and did not
//! build, and it is smaller than it looks: every desktop environment worth
//! naming on Linux reads the same directory, and the specification for it is
//! eleven years old and stable.
//!
//! ## Why a file rather than a service
//!
//! A systemd user unit would also work, and would be wrong here. Autostart is
//! `~/.config/autostart/*.desktop` on GNOME, KDE, XFCE, Cinnamon, MATE, LXQt
//! and every tiling compositor that bothers — one mechanism, and the one a
//! user already knows how to look in. A unit file would be a second mechanism
//! that works on the subset of those running systemd `--user`, and would leave
//! the widget host as the one thing in somebody's session they cannot turn off
//! from their settings panel.
//!
//! ## What is refused
//!
//! Only what this command wrote is what this command will delete. The entry
//! carries `X-Uniwidged-Managed=true`, and `disable` on a file without it says
//! so and stops — because somebody who hand-wrote an autostart entry with
//! their own flags in it has said something, and silently replacing it would
//! be this program deciding it knows better.

use std::path::PathBuf;

/// The name of the entry, fixed: a second file would be a second copy of the
/// desktop running, and the whole design is one process.
const ENTRY: &str = "uniwidged.desktop";

/// What marks an entry as ours, so that `disable` never deletes somebody's own.
const MANAGED: &str = "X-Uniwidged-Managed=true";

pub fn command(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("enable") => enable(),
        Some("disable") => disable(),
        None | Some("status") => status(),
        Some(other) => Err(format!(
            "`{other}` is not something autostart does. \
             Try `uniwidged autostart enable|disable|status`."
        )),
    }
}

/// Where the session looks.
///
/// `XDG_CONFIG_HOME` rather than `crate::paths::config_dir`, and the two are
/// deliberately different: uniwidged's own configuration follows
/// `UNIWIDGED_CONFIG` so that a test — or two desktops side by side — can be
/// pointed somewhere else, and the autostart directory belongs to the *session*
/// and must not move with it.
#[cfg(all(unix, not(target_os = "macos")))]
fn autostart_dir() -> Result<PathBuf, String> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg).join("autostart"));
    }
    match std::env::var_os("HOME") {
        Some(home) => Ok(PathBuf::from(home).join(".config/autostart")),
        None => Err("neither XDG_CONFIG_HOME nor HOME is set, so there is \
                     nowhere a session would look"
            .into()),
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn autostart_dir() -> Result<PathBuf, String> {
    // Named rather than guessed at. Windows is a registry `Run` key or a
    // shortcut in the Startup folder, and macOS is a `LaunchAgent` plist —
    // both are M5 and M6, and neither has a shell to start yet, so an
    // autostart that launched a binary which then refuses to run would be a
    // worse answer than this one.
    Err("autostart is only implemented for Linux sessions so far — \
         Windows is milestone M5 and macOS is M6, see ROADMAP.md"
        .into())
}

fn entry_path() -> Result<PathBuf, String> {
    Ok(autostart_dir()?.join(ENTRY))
}

/// The binary the entry should point at.
///
/// Resolved to an absolute path now rather than left as `uniwidged` on `PATH`:
/// a session's `PATH` is not a shell's, and an entry that works when it is
/// written and silently does nothing at login is the worst of the available
/// failures. `status` checks the recorded path still exists, which is what
/// catches a binary that moved afterwards.
fn executable() -> Result<PathBuf, String> {
    let path =
        std::env::current_exe().map_err(|e| format!("could not find this program on disk: {e}"))?;
    // `current_exe` can return a path through a deleted or replaced file on
    // some systems; canonicalising is what turns it into the one a session
    // would resolve.
    Ok(path.canonicalize().unwrap_or(path))
}

fn enable() -> Result<(), String> {
    let dir = autostart_dir()?;
    let path = dir.join(ENTRY);
    if let Some(existing) = read_entry(&path)?
        && !existing.contains(MANAGED)
    {
        return Err(format!(
            "{} already exists and was not written by uniwidged.\n  \
             Nothing has been changed. Edit it yourself, or delete it and run \
             this again.",
            path.display()
        ));
    }
    let exe = executable()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(&path, entry(&exe)).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("  uniwidged will start with your session");
    println!("  {}", path.display());
    println!("  running {} run", exe.display());
    Ok(())
}

fn disable() -> Result<(), String> {
    let path = entry_path()?;
    match read_entry(&path)? {
        None => {
            println!("  uniwidged was not set to start with your session");
            Ok(())
        }
        Some(text) if !text.contains(MANAGED) => Err(format!(
            "{} was not written by uniwidged, so it has been left alone.\n  \
             Delete it yourself if that is what you meant.",
            path.display()
        )),
        Some(_) => {
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            println!("  uniwidged will no longer start with your session");
            println!("  removed {}", path.display());
            // Said, because the two are separate and people expect otherwise.
            println!("  your widgets, their placements and their grants are untouched");
            Ok(())
        }
    }
}

fn status() -> Result<(), String> {
    let path = entry_path()?;
    let Some(text) = read_entry(&path)? else {
        println!("  uniwidged does not start with your session");
        println!("  `uniwidged autostart enable` changes that");
        return Ok(());
    };
    if !text.contains(MANAGED) {
        println!(
            "  {} exists and was not written by uniwidged",
            path.display()
        );
        println!("  it is left alone; `disable` will refuse to delete it");
        return Ok(());
    }
    println!("  uniwidged starts with your session");
    println!("  {}", path.display());
    match exec_line(&text) {
        Some(exec) => {
            // The failure this exists to catch: the entry is fine and points
            // at a binary that has been moved, upgraded out from under it, or
            // built into a target directory that was cleaned. At login that is
            // silence, which is the hardest kind of thing to debug.
            let program = exec.split_whitespace().next().unwrap_or("");
            let exists = std::path::Path::new(program).exists();
            println!("  running {exec}");
            if !exists {
                println!(
                    "  but {program} is not there any more — \
                     `uniwidged autostart enable` points it at this one"
                );
            }
        }
        None => println!("  but it has no Exec line, which is not a working entry"),
    }
    Ok(())
}

fn read_entry(path: &std::path::Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn exec_line(text: &str) -> Option<&str> {
    text.lines()
        .find_map(|line| line.strip_prefix("Exec="))
        .map(str::trim)
}

/// The entry itself.
///
/// `Terminal=false` and `NoDisplay=true` because this is a background process
/// and not something to put in an applications menu.
/// `X-GNOME-Autostart-enabled=true` is read by GNOME and ignored elsewhere, and
/// costs one line. There is deliberately no `OnlyShowIn`: the host picks its
/// window system at run time, so an entry restricted to one desktop
/// environment would be a lie the moment somebody logs into another.
fn entry(exe: &std::path::Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=uniwidged\n\
         Comment=Desktop widgets\n\
         Exec={} run\n\
         Terminal=false\n\
         NoDisplay=true\n\
         X-GNOME-Autostart-enabled=true\n\
         {MANAGED}\n",
        exe.display()
    )
}
