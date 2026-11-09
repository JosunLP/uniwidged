//! Starting with the session.
//!
//! A desktop widget that has to be launched by hand is a desktop widget
//! somebody stops using in a week. This is the piece M7 named and did not
//! build, and it is smaller than it looks: every platform has exactly one
//! place a session looks, and each of them is documented and stable.
//!
//! | | where | what marks it as ours |
//! |---|---|---|
//! | Linux | `~/.config/autostart/uniwidged.desktop` | `X-Uniwidged-Managed=true` |
//! | macOS | `~/Library/LaunchAgents/io.github.josunlp.uniwidged.plist` | an `X-Uniwidged-Managed` comment |
//! | Windows | `HKCU\…\CurrentVersion\Run`, value `uniwidged` | the `--background` flag on the command |
//!
//! ## Why a file rather than a service
//!
//! A systemd user unit would also work on Linux, and would be wrong here.
//! Autostart is `~/.config/autostart/*.desktop` on GNOME, KDE, XFCE, Cinnamon,
//! MATE, LXQt and every tiling compositor that bothers — one mechanism, and the
//! one a user already knows how to look in. A unit file would be a second
//! mechanism that works on the subset of those running systemd `--user`, and
//! would leave the widget host as the one thing in somebody's session they
//! cannot turn off from their settings panel. The same argument picks the
//! `Run` key over a scheduled task on Windows, and a `LaunchAgent` over a login
//! item on macOS: each is what *Settings → Startup apps* and *Login Items*
//! list, and therefore what a user can switch off without knowing this exists.
//!
//! ## What is refused
//!
//! Only what this command wrote is what this command will delete. Somebody who
//! hand-wrote an autostart entry with their own flags in it has said something,
//! and silently replacing it would be this program deciding it knows better.

use std::path::{Path, PathBuf};

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
    // would resolve. Not on Windows, where it would add a `\\?\` prefix that
    // the shell's `Run` key does not understand.
    #[cfg(not(windows))]
    let path = path.canonicalize().unwrap_or(path);
    Ok(path)
}

fn enable() -> Result<(), String> {
    let where_ = platform::location()?;
    if let Some(existing) = platform::read()?
        && !platform::is_ours(&existing)
    {
        return Err(format!(
            "{where_} already exists and was not written by uniwidged.\n  \
             Nothing has been changed. Edit it yourself, or delete it and run \
             this again."
        ));
    }
    let exe = executable()?;
    platform::write(&platform::entry(&exe))?;
    println!("  uniwidged will start with your session");
    println!("  {where_}");
    println!("  running {} run", exe.display());
    Ok(())
}

fn disable() -> Result<(), String> {
    let where_ = platform::location()?;
    match platform::read()? {
        None => {
            println!("  uniwidged was not set to start with your session");
            Ok(())
        }
        Some(text) if !platform::is_ours(&text) => Err(format!(
            "{where_} was not written by uniwidged, so it has been left alone.\n  \
             Delete it yourself if that is what you meant."
        )),
        Some(_) => {
            platform::remove()?;
            println!("  uniwidged will no longer start with your session");
            println!("  removed {where_}");
            // Said, because the two are separate and people expect otherwise.
            println!("  your widgets, their placements and their grants are untouched");
            Ok(())
        }
    }
}

fn status() -> Result<(), String> {
    let where_ = platform::location()?;
    let Some(text) = platform::read()? else {
        println!("  uniwidged does not start with your session");
        println!("  `uniwidged autostart enable` changes that");
        return Ok(());
    };
    if !platform::is_ours(&text) {
        println!("  {where_} exists and was not written by uniwidged");
        println!("  it is left alone; `disable` will refuse to delete it");
        return Ok(());
    }
    println!("  uniwidged starts with your session");
    println!("  {where_}");
    match platform::program(&text) {
        Some(program) => {
            // The failure this exists to catch: the entry is fine and points
            // at a binary that has been moved, upgraded out from under it, or
            // built into a target directory that was cleaned. At login that is
            // silence, which is the hardest kind of thing to debug.
            println!("  running {} run", program.display());
            if !program.exists() {
                println!(
                    "  but {} is not there any more — \
                     `uniwidged autostart enable` points it at this one",
                    program.display()
                );
            }
        }
        None => println!("  but it names no program, which is not a working entry"),
    }
    Ok(())
}

#[cfg(unix)]
fn read_file(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(unix)]
fn write_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::*;

    /// The name of the entry, fixed: a second file would be a second copy of
    /// the desktop running, and the whole design is one process.
    const ENTRY: &str = "uniwidged.desktop";

    /// What marks an entry as ours, so that `disable` never deletes
    /// somebody's own.
    const MANAGED: &str = "X-Uniwidged-Managed=true";

    /// Where the session looks.
    ///
    /// `XDG_CONFIG_HOME` rather than `crate::paths::config_dir`, and the two
    /// are deliberately different: uniwidged's own configuration follows
    /// `UNIWIDGED_CONFIG` so that a test — or two desktops side by side — can
    /// be pointed somewhere else, and the autostart directory belongs to the
    /// *session* and must not move with it.
    fn path() -> Result<PathBuf, String> {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return Ok(PathBuf::from(xdg).join("autostart").join(ENTRY));
        }
        match std::env::var_os("HOME") {
            Some(home) => Ok(PathBuf::from(home).join(".config/autostart").join(ENTRY)),
            None => Err("neither XDG_CONFIG_HOME nor HOME is set, so there is \
                         nowhere a session would look"
                .into()),
        }
    }

    pub(super) fn location() -> Result<String, String> {
        Ok(path()?.display().to_string())
    }

    pub(super) fn read() -> Result<Option<String>, String> {
        read_file(&path()?)
    }

    pub(super) fn write(text: &str) -> Result<(), String> {
        write_file(&path()?, text)
    }

    pub(super) fn remove() -> Result<(), String> {
        let path = path()?;
        std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub(super) fn is_ours(text: &str) -> bool {
        text.contains(MANAGED)
    }

    pub(super) fn program(text: &str) -> Option<PathBuf> {
        let exec = text.lines().find_map(|line| line.strip_prefix("Exec="))?;
        exec.split_whitespace().next().map(PathBuf::from)
    }

    /// The entry itself.
    ///
    /// `Terminal=false` and `NoDisplay=true` because this is a background
    /// process and not something to put in an applications menu.
    /// `X-GNOME-Autostart-enabled=true` is read by GNOME and ignored
    /// elsewhere, and costs one line. There is deliberately no `OnlyShowIn`:
    /// the host picks its window system at run time, so an entry restricted to
    /// one desktop environment would be a lie the moment somebody logs into
    /// another.
    pub(super) fn entry(exe: &Path) -> String {
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
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    const LABEL: &str = "io.github.josunlp.uniwidged";
    const MANAGED: &str = "<!-- X-Uniwidged-Managed=true -->";

    /// `~/Library/LaunchAgents`, read by `launchd` at login. `HOME` rather
    /// than the configuration directory, for the reason the Linux entry
    /// gives: this belongs to the session.
    fn path() -> Result<PathBuf, String> {
        match std::env::var_os("HOME") {
            Some(home) => Ok(PathBuf::from(home)
                .join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist"))),
            None => Err("HOME is not set, so there is nowhere launchd would look".into()),
        }
    }

    pub(super) fn location() -> Result<String, String> {
        Ok(path()?.display().to_string())
    }

    pub(super) fn read() -> Result<Option<String>, String> {
        read_file(&path()?)
    }

    pub(super) fn write(text: &str) -> Result<(), String> {
        write_file(&path()?, text)
    }

    pub(super) fn remove() -> Result<(), String> {
        let path = path()?;
        std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub(super) fn is_ours(text: &str) -> bool {
        text.contains(MANAGED)
    }

    /// The first `<string>` of `ProgramArguments`, which is the program.
    pub(super) fn program(text: &str) -> Option<PathBuf> {
        let after = text.split("<key>ProgramArguments</key>").nth(1)?;
        let start = after.find("<string>")? + "<string>".len();
        let end = after[start..].find("</string>")? + start;
        Some(PathBuf::from(unescape(&after[start..end])))
    }

    fn escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn unescape(text: &str) -> String {
        text.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
    }

    /// A `LaunchAgent` that runs the desktop once at login.
    ///
    /// `RunAtLoad` and no `KeepAlive`: a desktop that exits has been closed or
    /// has said why in its log, and `launchd` restarting it in a loop would
    /// bury that line under a hundred more. `ProcessType` `Interactive`
    /// because it draws on the screen and must not be throttled like a
    /// background daemon.
    pub(super) fn entry(exe: &Path) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             {MANAGED}\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \t<key>Label</key>\n\
             \t<string>{LABEL}</string>\n\
             \t<key>ProgramArguments</key>\n\
             \t<array>\n\
             \t\t<string>{}</string>\n\
             \t\t<string>run</string>\n\
             \t</array>\n\
             \t<key>RunAtLoad</key>\n\
             \t<true/>\n\
             \t<key>ProcessType</key>\n\
             \t<string>Interactive</string>\n\
             </dict>\n\
             </plist>\n",
            escape(&exe.display().to_string())
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_program_is_read_back_from_the_entry() {
            let exe = Path::new("/Applications/Uni & Widged.app/Contents/MacOS/uniwidged");
            assert_eq!(program(&entry(exe)).as_deref(), Some(exe));
            assert!(is_ours(&entry(exe)));
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };

    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "uniwidged";
    /// What every command this writes ends with, and what a hand-written one
    /// would have no reason to: the desktop's own flag for being started by
    /// the session rather than a person. See `command_run`.
    const MANAGED: &str = " run --background";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn location() -> Result<String, String> {
        Ok(format!(r"HKEY_CURRENT_USER\{KEY}\{VALUE}"))
    }

    pub(super) fn read() -> Result<Option<String>, String> {
        let (key, value) = (wide(KEY), wide(VALUE));
        let mut buffer = vec![0u16; 2048];
        let mut bytes = (buffer.len() * 2) as u32;
        // SAFETY: names that outlive the call, and a buffer of the size given.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != 0 {
            return Err(format!(
                "reading {}: {}",
                location()?,
                std::io::Error::from_raw_os_error(status as i32)
            ));
        }
        let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        Ok(Some(String::from_utf16_lossy(&buffer[..len])))
    }

    pub(super) fn write(text: &str) -> Result<(), String> {
        let (key, value, data) = (wide(KEY), wide(VALUE), wide(text));
        // SAFETY: names and data that outlive the call; the length includes
        // the terminating null, as `REG_SZ` requires.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if status != 0 {
            return Err(format!(
                "writing {}: {}",
                location()?,
                std::io::Error::from_raw_os_error(status as i32)
            ));
        }
        Ok(())
    }

    pub(super) fn remove() -> Result<(), String> {
        let (key, value) = (wide(KEY), wide(VALUE));
        // SAFETY: names that outlive the call.
        let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
        if status != 0 && status != ERROR_FILE_NOT_FOUND {
            return Err(format!(
                "removing {}: {}",
                location()?,
                std::io::Error::from_raw_os_error(status as i32)
            ));
        }
        Ok(())
    }

    pub(super) fn is_ours(text: &str) -> bool {
        text.ends_with(MANAGED)
    }

    /// The quoted program at the start of the command.
    pub(super) fn program(text: &str) -> Option<PathBuf> {
        let rest = text.strip_prefix('"')?;
        rest.split('"').next().map(PathBuf::from)
    }

    /// The program quoted, because it is usually under `Program Files` and an
    /// unquoted path with a space in it is a different program.
    pub(super) fn entry(exe: &Path) -> String {
        format!("\"{}\"{MANAGED}", exe.display())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_path_with_spaces_survives_the_round_trip() {
            let exe = Path::new(r"C:\Program Files\uniwidged\uniwidged.exe");
            let text = entry(exe);
            assert!(is_ours(&text));
            assert_eq!(program(&text).as_deref(), Some(exe));
        }
    }
}
