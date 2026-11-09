//! The desktop, on whichever window system this is.
//!
//! This is `ARCHITECTURE.md` §5's loop with a real window system under it:
//!
//! ```text
//! loop {
//!     let timeout = host.next_timeout(now);   // None ⇒ block
//!     window.wait(timeout);                   // costs nothing
//!     deliver whatever arrived
//!     host.tick(now, wall);
//! }
//! ```
//!
//! One process, one GPU device, one loop, N widgets — each with its own
//! desktop-layer window and its own swapchain, all rendered from the same
//! device.
//!
//! **Both window systems.** A Wayland session gets a `wlr-layer-shell` surface
//! on the bottom layer; an X11 session gets a 32-bit `_NET_WM_STATE_BELOW`
//! window. Which one happened is in the start-up log, along with anything the
//! session could not provide. Everything below the first twenty lines is the
//! same either way — `uniwidged_shell::platform::linux` is where the two meet.
//!
//! **And the other two platforms.** Windows gets a `WS_EX_NOREDIRECTIONBITMAP`
//! popup kept at `HWND_BOTTOM`, macOS a non-activating panel at desktop-icon
//! level. Nothing in this file names either: it is written against
//! `uniwidged_shell::platform::desktop`, which is the same set of names on all
//! four, and the only lines here that know which platform they are on are
//! [`open_link`] and [`doctor`] — because what opens a browser, and what is
//! worth reporting about a session, are genuinely different questions on each.

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use uniwidged_compose::Theme;
use uniwidged_core::geom::PhysicalRect;
use uniwidged_core::{Point, WidgetId};
use uniwidged_host::config::Config;
use uniwidged_host::interaction::{Interaction, Press, Release};
use uniwidged_host::placement;
use uniwidged_host::{Activated, Host};
use uniwidged_render::SurfaceId;
use uniwidged_runtime::manifest::Manifest;
use uniwidged_shell::platform::desktop::{self, DesktopWindow};
use uniwidged_shell::{Input, Output, WakePipe};

use crate::log::{problem, say};

/// One widget on the desktop: what it is, where it draws, and whether the user
/// has pinned it down.
struct Running {
    id: WidgetId,
    name: String,
    /// Index into the window list, which owns the windows: dropping one
    /// destroys the surface wgpu is drawing into, so the list outlives every
    /// use of the surface.
    ///
    /// Not stable across a removal — [`stop_widget`] closes the gap and fixes
    /// up every index above it, which is cheap at the number of widgets a
    /// desktop has and leaves no empty slots to reason about.
    window: usize,
    surface: SurfaceId,
    locked: bool,
    /// What it was started from.
    ///
    /// Kept so that a reload can notice the file underneath a running widget
    /// has been replaced — which is what `uniwidged install` over an existing
    /// widget and `uniwidged update` both do, and which is invisible from the
    /// name alone.
    component: std::path::PathBuf,
}

/// Every widget on the screen, and the windows they draw into.
///
/// One struct rather than three parallel lists, because installing and removing
/// a widget while the desktop runs has to keep them consistent and three
/// arguments in that order is three chances to pass them in another.
struct Desktop {
    /// Owns every window for as long as anything draws into one: dropping a
    /// window destroys the surface wgpu is presenting to, so this outlives the
    /// renderer's surface for it.
    windows: Vec<DesktopWindow>,
    running: Vec<Running>,
    /// Which widget an event on a window belongs to.
    ///
    /// Holds a [`WidgetId`] rather than an index into `running`, and that is
    /// the whole reason this map is worth its allocation: an index stops
    /// meaning what it did the moment a widget is removed, and a stale index
    /// does not fail — it routes a click to somebody else's widget.
    by_window: HashMap<u32, WidgetId>,
}

impl Desktop {
    fn new() -> Self {
        Desktop {
            windows: Vec::new(),
            running: Vec::new(),
            by_window: HashMap::new(),
        }
    }

    fn index_of(&self, id: WidgetId) -> Option<usize> {
        self.running.iter().position(|widget| widget.id == id)
    }

    fn index_named(&self, name: &str) -> Option<usize> {
        self.running.iter().position(|widget| widget.name == name)
    }

    fn index_for_window(&self, window: u32) -> Option<usize> {
        self.by_window
            .get(&window)
            .and_then(|id| self.index_of(*id))
    }
}

/// Compiles a widget, gives it a window and a swapchain, and starts it.
///
/// Called once per widget at start-up and again for every widget that appears
/// in the configuration while the desktop is running. That it is *one* function
/// is the point: a second copy for the reload path would be a second copy to
/// forget the alpha-mode warning in, or the settings, or the order the surface
/// and the window have to be created in.
///
/// Nothing here is fatal to the desktop when it is called from a reload — the
/// caller decides, because at start-up a widget that will not start is a reason
/// to stop and say so, and on a running desktop it is a reason to print a line
/// and leave the other widgets alone.
fn start_widget(
    host: &mut Host,
    desktop: &mut Desktop,
    entry: &uniwidged_host::config::WidgetConfig,
) -> Result<usize, String> {
    let wasm = std::fs::read(&entry.component)
        .map_err(|e| format!("{}: {e}", entry.component.display()))?;
    let manifest_text = std::fs::read_to_string(entry.manifest_path())
        .map_err(|e| format!("{}: {e}", entry.manifest_path().display()))?;
    let manifest = Manifest::parse(&manifest_text).map_err(|e| e.to_string())?;
    let policy = entry.policy(&manifest).map_err(|e| e.to_string())?;

    let placement = &entry.placement;
    let geometry = PhysicalRect::new(placement.x, placement.y, placement.width, placement.height);
    let window = DesktopWindow::open(geometry, &entry.name).map_err(|e| e.to_string())?;
    if !window.has_compositor() {
        problem!(
            "no compositing manager is running, so `{}` will be \
             an opaque rectangle rather than a widget",
            entry.name
        );
    }

    // The user's own values, resolved against this manifest before the widget
    // is asked for a view. `install` starts every widget at its declared
    // defaults, so without this a configuration would be a file nothing reads.
    let (values, complaints) = entry.settings(&manifest);
    let id = host
        .install(&wasm, manifest, policy)
        .map_err(|e| e.to_string())?;
    if let Err(e) = host.configure(id, &values) {
        problem!("could not configure `{}`: {e}", entry.name);
    }
    for complaint in complaints {
        // A line in somebody's configuration that is being ignored has to be
        // findable. `uniwidged configure <name>` says the same thing without
        // starting a desktop.
        problem!("`{}`: {complaint}", entry.name);
    }
    let (display, handle) = window.raw_handles();
    // SAFETY: the window is kept in `desktop.windows` for as long as the
    // surface exists — `stop_widget` removes the surface first — and was
    // configured and mapped above.
    let surface = unsafe {
        host.renderer_mut()
            .add_swapchain_surface(display, handle, geometry.width, geometry.height)
    }
    .map_err(|e| e.to_string())?;
    host.attach(id, surface).map_err(|e| e.to_string())?;

    if host.renderer_mut().alpha_mode(surface)
        == Some(uniwidged_render::wgpu::CompositeAlphaMode::Opaque)
    {
        problem!(
            "this driver offers no transparent swapchain, so `{}` \
             will sit on a solid rectangle — see ARCHITECTURE.md §10.1",
            entry.name
        );
    }

    desktop.by_window.insert(window.window_id(), id);
    let index = desktop.running.len();
    desktop.running.push(Running {
        id,
        name: entry.name.clone(),
        window: desktop.windows.len(),
        surface,
        locked: placement.locked,
        component: entry.component.clone(),
    });
    desktop.windows.push(window);
    Ok(index)
}

/// Takes a widget off the desktop: its instance, its surface and its window.
///
/// **The order is the whole of it**, and it is the reason this is not three
/// lines at the call site:
///
/// 1. [`Host::remove`] first, which detaches the surface from the renderer.
///    While a surface is attached the next frame composes into it.
/// 2. The window after, because dropping it destroys what wgpu was presenting
///    to. Reversed, the renderer would hold a surface over a window the X
///    server has already forgotten.
///
/// Removing from the middle of `windows` shifts every later index, so the
/// entries that refer to them are fixed up here rather than left to be
/// discovered — at the number of windows a desktop has, closing the gap costs
/// nothing and leaves no empty slot to reason about.
fn stop_widget(host: &mut Host, desktop: &mut Desktop, index: usize) {
    let widget = desktop.running.remove(index);
    if let Err(e) = host.remove(widget.id) {
        // Only reachable if the host and this list disagree about what is
        // running, which one thread cannot arrange. Said rather than ignored,
        // because if it ever happens the window is about to go anyway.
        problem!("could not stop `{}`: {e}", widget.name);
    }
    let window = desktop.windows.remove(widget.window);
    desktop.by_window.remove(&window.window_id());
    drop(window);
    for other in &mut desktop.running {
        if other.window > widget.window {
            other.window -= 1;
        }
    }
}

/// Puts a widget where its placement says, against the monitors that are here.
///
/// Separate from [`start_widget`] because at start-up the two cannot happen
/// together: Wayland reports its outputs through a connection, and a connection
/// here means a surface — so the first window has to exist before anything
/// knows what the monitors are called.
fn place_widget(
    host: &mut Host,
    desktop: &mut Desktop,
    index: usize,
    entry: &uniwidged_host::config::WidgetConfig,
    outputs: &[Output],
    announce: bool,
) -> Result<(), String> {
    let resolved = placement::resolve(&entry.placement, outputs);
    if resolved.displaced {
        problem!(
            "`{}` belongs on {}, which is not connected; it is \
             on {} for now and will go back when that monitor returns",
            entry.name,
            entry
                .placement
                .output
                .as_deref()
                .unwrap_or("another monitor"),
            resolved.output.as_deref().unwrap_or("this one"),
        );
    }
    let window = desktop.running[index].window;
    if resolved.rect != desktop.windows[window].geometry() {
        desktop.windows[window]
            .set_geometry(resolved.rect)
            .map_err(|e| e.to_string())?;
        host.renderer_mut()
            .resize(
                desktop.running[index].surface,
                resolved.rect.width,
                resolved.rect.height,
            )
            .map_err(|e| e.to_string())?;
        host.redraw(desktop.running[index].id);
    }
    if announce {
        let at = desktop.windows[window].geometry();
        say!(
            "  {} at {},{} {}x{}{}",
            entry.name,
            at.x,
            at.y,
            at.width,
            at.height,
            resolved
                .output
                .as_deref()
                .map(|o| format!(" on {o}"))
                .unwrap_or_default(),
        );
    }
    Ok(())
}

/// Hands a URL to whatever this desktop opens web pages with.
///
/// `xdg-open`, spawned and **not waited for**: it may start a browser that
/// takes seconds, and the desktop's loop is what draws every widget on the
/// screen. Its output goes nowhere for the same reason — a browser's warnings
/// on somebody's terminal are not this program's news to deliver.
///
/// The URL has already been checked twice by the host: that it is an `https`
/// URL at all, and that the widget was granted `link:` for its host. Nothing
/// here re-reads it, because a third reading is a third chance to disagree —
/// see `uniwidged_runtime::net::host_of`, which is one function for exactly
/// that reason.
fn open_link(url: &str) {
    match spawn_browser(url) {
        Ok(()) => say!("  opening {url}"),
        Err(e) => problem!("could not open {url}: {e}"),
    }
}

/// `xdg-open` on Linux, `open` on macOS: executed directly with the URL as one
/// argument — no shell — so nothing in it can become a second command, the
/// same rule the `command` capability follows.
#[cfg(unix)]
fn spawn_browser(url: &str) -> Result<(), String> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

/// `ShellExecuteW` with the `open` verb: what Explorer does with a URL, and
/// the one way to reach the user's default browser that involves no command
/// line to quote. It returns as soon as the request is handed over.
#[cfg(windows)]
fn spawn_browser(url: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(std::iter::once(0)).collect() };
    let verb = wide("open");
    let file = wide(url);
    // SAFETY: two strings that outlive the call, and no window, parameters
    // or directory.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Documented: anything above 32 is success.
    if result as usize > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecuteW answered {}", result as usize))
    }
}

/// Whether the widget running under this name is no longer the one the
/// configuration describes.
///
/// Two ways that happens, and both are one command somebody ran:
///
/// * **The component moved.** `uniwidged install` over an existing widget
///   keeps its placement and its settings and points the entry at a new file.
/// * **The version changed.** `uniwidged update` writes a new component to the
///   same path in the widget store, so the path proves nothing and the manifest
///   beside it is what has to be read.
///
/// A manifest that will not parse answers `false` rather than `true`: a file
/// somebody is halfway through editing is not a reason to restart a widget that
/// is working. It becomes an error at the point something tries to start it.
fn was_replaced(
    host: &Host,
    widget: &Running,
    entry: &uniwidged_host::config::WidgetConfig,
) -> bool {
    if widget.component != entry.component {
        return true;
    }
    let Some(running) = host.widget(widget.id) else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(entry.manifest_path()) else {
        return false;
    };
    match Manifest::parse(&text) {
        Ok(manifest) => manifest.version != running.manifest().version,
        Err(_) => false,
    }
}

/// Reads the configuration again and applies it to a running desktop.
///
/// What a `SIGHUP` does. The split is the one `ARCHITECTURE.md` §5 makes about
/// surfaces: [`uniwidged_host::reload::apply`] settles what the *widgets* do —
/// which is settings, because a setting is a value behind a shared handle —
/// and this settles what the *windows* do, because only this layer owns them.
///
/// ## Widgets that arrived and widgets that left
///
/// Until this did them, both were *reported* and the user was told to restart.
/// That was the honest answer while there was no [`Host::remove`]: a widget
/// leaving needs its surface detached before its window is destroyed, and a
/// widget arriving needs a component compiled, a window created, a swapchain
/// attached and a placement resolved — and doing half of either on a signal is
/// how a desktop ends up with a window nothing draws into.
///
/// So the order is the feature. [`stop_widget`] and [`start_widget`] are the
/// same two functions the desktop starts with, and each does the whole of its
/// half or none of it.
///
/// **Removals happen before additions**, which is not arbitrary: a widget
/// upgraded in place is a removal and an addition of the same name, and doing
/// them the other way round would have two windows titled the same and two
/// instances writing the same data directory for as long as it took.
///
/// Everything is best effort and nothing here can stop the desktop. A widget
/// that will not start is one line on the terminal; at start-up the same
/// failure is fatal, because a desktop that comes up with nothing on it should
/// say why rather than sit there. A configuration that has become unreadable is
/// a reason to keep running with the one already loaded, not a reason to take
/// somebody's widgets off their screen.
fn reload(
    host: &mut Host,
    config_path: &Path,
    desktop: &mut Desktop,
    outputs: &[Output],
) -> Option<Config> {
    let config = match Config::read(config_path) {
        Ok(config) => config,
        Err(e) => {
            problem!(
                "{} is not readable, so nothing changed: {e}",
                config_path.display()
            );
            return None;
        }
    };

    let applied = uniwidged_host::reload::apply(host, &config);
    for complaint in &applied.complaints {
        problem!("{complaint}");
    }
    for id in &applied.reconfigured {
        // Named, because "the setting took effect" is the fact somebody
        // changing one is waiting for — and because a widget that answered
        // that its view is unchanged still had its configuration applied.
        if let Some(widget) = host.widget(*id) {
            say!("  `{}` reconfigured", widget.name);
        }
    }

    let mut replaced = 0;

    // Gone from the file, or switched off in it. First, for the reason in the
    // doc comment above.
    for name in &applied.removed {
        let Some(index) = desktop.index_named(name) else {
            continue;
        };
        stop_widget(host, desktop, index);
        say!("  `{name}` stopped");
    }

    // Still in the file under the same name, and no longer the same widget.
    // Stopped and started rather than reloaded in place: a new version is a
    // new component, a new manifest and possibly a new set of settings, and
    // there is no half of that worth doing separately. What it keeps is what
    // the file keeps — where it sits, what it was configured with, and whether
    // it is switched off.
    for index in (0..desktop.running.len()).rev() {
        let Some(entry) = config.widget(&desktop.running[index].name).cloned() else {
            continue;
        };
        if !was_replaced(host, &desktop.running[index], &entry) {
            continue;
        }
        stop_widget(host, desktop, index);
        match start_widget(host, desktop, &entry) {
            Ok(fresh) => {
                if let Err(e) = place_widget(host, desktop, fresh, &entry, outputs, false) {
                    problem!("could not place `{}`: {e}", entry.name);
                }
                say!("  `{}` replaced with a new version", entry.name);
            }
            // The old one is already gone — it had to be, or two instances
            // would be writing the same data directory. Saying so is all that
            // is left, and it is better than a desktop that refuses to apply
            // any of the reload because one widget's new version is broken.
            Err(e) => problem!(
                "`{}` was replaced by a version that does not \
                 start, and is now stopped: {e}",
                entry.name
            ),
        }
        replaced += 1;
    }

    // And in the file, enabled, and not running.
    for name in &applied.added {
        let Some(entry) = config.widget(name) else {
            continue;
        };
        match start_widget(host, desktop, entry) {
            Ok(index) => {
                if let Err(e) = place_widget(host, desktop, index, entry, outputs, false) {
                    problem!("could not place `{name}`: {e}");
                }
                let at = desktop.windows[desktop.running[index].window].geometry();
                say!(
                    "  `{name}` started at {},{} {}x{}",
                    at.x,
                    at.y,
                    at.width,
                    at.height
                );
            }
            // The desktop keeps running. A widget whose component has gone
            // missing, whose manifest no longer parses or which asks for a
            // capability the file does not grant is a widget that does not
            // start, and the others are none of its business.
            Err(e) => problem!("`{name}` did not start: {e}"),
        }
    }

    // Placement, which the host cannot do: it does not own windows. Counted,
    // because "nothing changed" is a line about the *whole* reload and the
    // host's half cannot see this one — a live run printed it over a window it
    // had just moved.
    let mut moved = 0;
    for index in 0..desktop.running.len() {
        let Some(widget) = config.widget(&desktop.running[index].name) else {
            continue;
        };
        let resolved = placement::resolve(&widget.placement, outputs);
        let window = desktop.running[index].window;
        if resolved.rect != desktop.windows[window].geometry() {
            let name = desktop.running[index].name.clone();
            if let Err(e) = apply_geometry(host, desktop, index, resolved.rect) {
                problem!("could not move `{name}`: {e}");
                continue;
            }
            say!(
                "  `{name}` moved to {},{} {}x{}",
                resolved.rect.x,
                resolved.rect.y,
                resolved.rect.width,
                resolved.rect.height
            );
            moved += 1;
        }
    }
    // The lock switch is this layer's too: it decides what a press means, and
    // the host never sees a pointer.
    let mut relocked = 0;
    for entry in desktop.running.iter_mut() {
        if let Some(widget) = config.widget(&entry.name)
            && entry.locked != widget.placement.locked
        {
            entry.locked = widget.placement.locked;
            say!(
                "  `{}` is {}",
                entry.name,
                if entry.locked {
                    "locked where it is"
                } else {
                    "free to move again"
                }
            );
            relocked += 1;
        }
    }

    if applied.is_quiet() && replaced == 0 && moved == 0 && relocked == 0 {
        say!("  configuration read again; nothing changed");
    }
    // Handed back so the loop keeps *this* as the configuration it works from.
    // It is the one the widgets on the screen now match, and two other things
    // read it: where a dragged widget's new position is written, and where
    // every placement is re-resolved when the monitors change. Leaving the
    // loop with the copy it started with would mean a widget installed since
    // then is in neither — dragged and not saved, and left behind when a
    // monitor is unplugged.
    Some(config)
}

/// What `uniwidged doctor` can say about this session without starting one.
/// One line per counter a widget granted `metrics` can read: the answer, or
/// the reason it would be told `unavailable`.
///
/// Separate from [`doctor`] so the wording is tested without a display.
pub(crate) fn machine_counters(counters: &uniwidged_runtime::services::Counters) -> Vec<String> {
    use uniwidged_runtime::machine::{PowerState, describe_rate};
    let percent = |fraction: f32| format!("{}%", (fraction * 100.0).round() as u32);
    let mut lines = Vec::new();
    lines.push(match counters.cpu_load() {
        // The first read on a fresh sampler is the average since boot, and
        // saying so stops somebody wondering why it is not this second's.
        Ok(load) => format!("cpu: {} since boot", percent(load)),
        Err(why) => format!("cpu: unavailable — {why}"),
    });
    lines.push(match counters.memory_load() {
        Ok(load) => format!("memory: {} in use", percent(load)),
        Err(why) => format!("memory: unavailable — {why}"),
    });
    lines.push(match counters.disk_load() {
        Ok(load) => format!(
            "disk: {} in use (the filesystem holding $HOME)",
            percent(load)
        ),
        Err(why) => format!("disk: unavailable — {why}"),
    });
    lines.push(match counters.battery() {
        // Not a failure, and worded so it does not read like one.
        Ok(None) => "battery: none — widgets are told this machine runs on mains".to_string(),
        Ok(Some(battery)) => {
            let state = match battery.state {
                PowerState::Charging => "charging",
                PowerState::Discharging => "discharging",
                PowerState::Idle => "plugged in, not charging",
            };
            match battery.minutes_left {
                Some(minutes) => format!(
                    "battery: {}, {state}, {}:{:02} to go",
                    percent(battery.charge),
                    minutes / 60,
                    minutes % 60
                ),
                None => format!("battery: {}, {state}", percent(battery.charge)),
            }
        }
        Err(why) => format!("battery: unavailable — {why}"),
    });
    lines.push(match counters.network_throughput() {
        // A rate needs two samples, and `doctor` takes one. What it can say
        // is that the counters are there to be read.
        Ok(rate) => format!(
            "network: readable ({} in, {} out on a first read; a widget sees a rate from its second)",
            describe_rate(rate.received),
            describe_rate(rate.sent)
        ),
        Err(why) => format!("network: unavailable — {why}"),
    });
    lines
}

pub fn doctor() {
    session_doctor();
    let locale = uniwidged_runtime::services::Locale::system();
    println!(
        "  dates and times: {} (UTC{}{:02}:{:02})",
        locale.format_date(0),
        if locale.utc_offset_seconds < 0 {
            '-'
        } else {
            '+'
        },
        locale.utc_offset_seconds.abs() / 3600,
        (locale.utc_offset_seconds.abs() % 3600) / 60,
    );
    println!("  widget data: {}", crate::paths::data_root().display());
    // What a widget granted `metrics` would be told right now, counter by
    // counter. The question a widget author asks when their meter shows a
    // dash: is it my code, or does this machine not have one?
    println!("  machine counters (what `metrics` answers here):");
    for line in machine_counters(&uniwidged_runtime::services::Counters::new()) {
        println!("    {line}");
    }
    // What a widget author needs when their widget reports `unavailable` and
    // they are trying to work out whether that is their manifest, their user's
    // choice, or this host.
    match uniwidged_host::Host::new(
        uniwidged_compose::Theme::default(),
        Duration::from_millis(16),
    ) {
        Ok(mut host) => {
            host.set_data_root(crate::paths::data_root());
            host.set_keyring(std::sync::Arc::new(uniwidged_runtime::FileKeyring::system(
                crate::paths::keyring_dir(),
            )));
            if let Some(what) = host.keyring_description() {
                // Two very different things are both "secrets are served", and
                // a user deciding whether to let a widget keep a password is
                // entitled to know which one this is.
                println!("  keychain: {what}");
            }
            let (served, unserved): (Vec<_>, Vec<_>) =
                host.capabilities().into_iter().partition(|(_, yes)| *yes);
            println!(
                "  capabilities served: {}",
                served
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            // Empty on a desktop now that the keychain is served: every
            // capability has something behind it. Not empty on a host that was
            // given no data root and no keychain — which is what `snapshot`
            // and every headless test are — so the line still has to exist and
            // still has to say what the answer means.
            if unserved.is_empty() {
                println!("  every capability this build knows about is served");
            } else {
                println!(
                    "  not yet: {} — a widget granted one of these is told \
                     `unavailable`, which is not the same as being refused",
                    unserved
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        Err(e) => println!("  capabilities: cannot say — {e}"),
    }
    monitors_doctor();
}

/// Which session this is, and what kind of surface a widget would get in it.
#[cfg(all(unix, not(target_os = "macos")))]
fn session_doctor() {
    use uniwidged_shell::platform::linux::{self, Session};
    match linux::session() {
        Session::Wayland => {
            let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
            println!("  session: Wayland ({display})");
            // Opening a real surface is the only way to find out whether this
            // compositor has a layer shell, and it is the question that decides
            // whether a widget is a widget here.
            match DesktopWindow::open(PhysicalRect::new(0, 0, 32, 32), "uniwidged doctor") {
                Ok(window) => {
                    println!("    shell: {}", window.describe());
                    let outputs = window.outputs();
                    if outputs.is_empty() {
                        println!("    monitors: this compositor reports no names");
                    } else {
                        println!("    monitors:");
                        for output in outputs {
                            println!(
                                "      {} {}x{} at {},{}",
                                output.name,
                                output.rect.width,
                                output.rect.height,
                                output.rect.x,
                                output.rect.y
                            );
                        }
                    }
                }
                Err(e) => println!("    cannot create a surface: {e}"),
            }
        }
        Session::X11 => println!("  session: X11"),
    }
}

/// The other two platforms have one window system each, and the question is
/// only whether a widget window can be made and what it says about itself.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn session_doctor() {
    match DesktopWindow::open(PhysicalRect::new(-200, -200, 32, 32), "uniwidged doctor") {
        Ok(window) => println!("  shell: {}", window.describe()),
        Err(e) => println!("  cannot create a surface: {e}"),
    }
}

/// The monitors, by the names placements are stored against.
#[cfg(all(unix, not(target_os = "macos")))]
fn monitors_doctor() {
    use uniwidged_shell::platform::linux::{self, Session};
    use uniwidged_shell::platform::x11::{AlphaPath, X11Window};
    if linux::session() == Session::Wayland {
        // The rest of this is about X11's transparency paths, which a Wayland
        // session does not have and does not need: `VK_COMPOSITE_ALPHA_PRE_
        // MULTIPLIED` is advertised there. Reporting on XWayland instead would
        // be reporting on a path this desktop is not taking.
        return;
    }
    // Placement is stored against these names, so a user who is wondering why a
    // widget is on the wrong screen needs to see what the server calls them.
    match uniwidged_shell::platform::x11::outputs(None) {
        Ok(outputs) if outputs.is_empty() => println!("  monitors: none reported"),
        Ok(outputs) => {
            println!("  monitors:");
            for output in outputs {
                println!(
                    "    {} {}x{} at {},{}{}",
                    output.name,
                    output.rect.width,
                    output.rect.height,
                    output.rect.x,
                    output.rect.y,
                    if output.primary { " (primary)" } else { "" },
                );
            }
        }
        Err(e) => println!(
            "  monitors: this display cannot list them ({e}); widgets are \
             placed at the coordinates they were saved with"
        ),
    }
    match X11Window::open(PhysicalRect::new(-200, -200, 32, 32), "uniwidged doctor") {
        Ok(window) => {
            println!("  32-bit TrueColor visual: yes");
            println!(
                "  compositing manager: {}",
                if window.has_compositor() {
                    "present"
                } else {
                    "ABSENT — the X server will ignore the alpha channel \
                     entirely, and widgets will be opaque rectangles"
                }
            );
            let caps = window.capabilities(AlphaPath::VulkanComposite);
            match caps.degradation() {
                None => println!("  everything a widget needs is available"),
                Some(lost) => println!("  degraded: {lost}"),
            }
        }
        Err(e) => println!("  cannot open a window: {e}"),
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn monitors_doctor() {
    let outputs = desktop::outputs();
    if outputs.is_empty() {
        println!("  monitors: none reported");
        return;
    }
    println!("  monitors:");
    for output in outputs {
        println!(
            "    {} {}x{} at {},{}{}",
            output.name,
            output.rect.width,
            output.rect.height,
            output.rect.x,
            output.rect.y,
            if output.primary { " (primary)" } else { "" },
        );
    }
}

/// Starts the desktop and does not return until it is closed.
pub fn run(config_path: &Path) -> Result<(), String> {
    let mut config = Config::read(config_path).map_err(|e| e.to_string())?;
    // Cloned rather than borrowed: a widget that is dragged writes its new
    // placement back into `config`, and a borrow of it would outlive the loop
    // that does so.
    let enabled: Vec<_> = config
        .widgets
        .iter()
        .filter(|w| w.enabled)
        .cloned()
        .collect();
    if enabled.is_empty() {
        return Err(format!(
            "nothing to run: {} lists no enabled widgets. \
             Install one with `uniwidged install <widget.wasm>`.",
            config_path.display()
        ));
    }

    // Opened before anything else says anything, so that the first line a
    // desktop writes is in the file too. Every `say!` and `problem!` below goes
    // to the terminal *and* here — and the terminal is the half that usually
    // does not exist, because a widget host is normally started with the
    // session. See `crate::log`.
    match crate::log::open() {
        Ok(path) => println!("  log: {}", path.display()),
        Err(e) => eprintln!("uniwidged: nothing will be logged to a file: {e}"),
    }

    // The loop blocks on its window system's socket with no timer at all when
    // nothing is scheduled. A widget's asynchronous work finishes on a worker
    // thread, and this is what lets it end that block — see
    // `uniwidged_shell::wake`.
    let wake = WakePipe::new().map_err(|e| format!("could not make a wake pipe: {e}"))?;
    // `SIGHUP` means *read your configuration again*, and the handler writes to
    // the pipe above rather than doing any work of its own — so an idle desktop
    // still blocks on its window system's socket with no timer at all. The
    // guard removes the pid file when this function returns.
    //
    // A desktop that cannot claim a pid file still runs. It just cannot be
    // told, which is a smaller loss than refusing to start.
    let _pid_file = match crate::daemon::listen(&wake) {
        Ok(guard) => {
            say!("  pid file: {}", guard.path().display());
            Some(guard)
        }
        Err(e) => {
            problem!("no pid file, so `uniwidged reload` will not find this desktop: {e}");
            None
        }
    };
    let mut host = Host::with_waker(
        Theme::default(),
        Duration::from_millis(16),
        uniwidged_runtime::tasks::Waker::with_notify(wake.notifier()),
    )
    .map_err(|e| e.to_string())?;
    // What is behind the `fs` gate. Each widget gets `<root>/<its name>`, and
    // the directory is created by its first write rather than at start-up: a
    // desktop should not leave an empty folder for every widget that never
    // stores anything.
    host.set_data_root(crate::paths::data_root());
    // What is behind the `secrets` gate. One store for the desktop, namespaced
    // per widget by the host — a widget has no argument through which to name
    // another's. Like the data directory, nothing is created until a widget
    // actually stores something.
    host.set_keyring(std::sync::Arc::new(uniwidged_runtime::FileKeyring::system(
        crate::paths::keyring_dir(),
    )));
    // Every widget's window, kept alive for as long as the desktop is: dropping
    // one destroys the window and, with it, the surface wgpu is drawing into.
    // X11 can be asked before a window exists; Wayland reports its outputs
    // through a connection, and a connection here means a surface. So widgets
    // are opened at the coordinates they were saved with and *then* placed —
    // which is one extra `set_geometry` on X11 and the only possible order on
    // Wayland.
    let mut outputs: Vec<Output> = desktop::outputs();

    let mut desktop = Desktop::new();

    // One widget that will not start must not take the others with it, and
    // that is a bug this loop had: a `?` here meant a deleted `.wasm`, a
    // manifest somebody had broken while editing, or a component built for the
    // wrong target emptied the whole screen and printed one line about one
    // widget. Which is the worst moment for a desktop to refuse to appear —
    // the user cannot see what else they had, and the thing they are missing
    // is the thing that would have told them.
    //
    // So each failure is named and the rest start. Only a desktop where
    // *nothing* started is an error, because then there is nothing to show and
    // saying so is all that is left.
    let mut refused: Vec<String> = Vec::new();
    for widget in &enabled {
        match start_widget(&mut host, &mut desktop, widget) {
            Ok(_) => {
                if desktop.windows.len() == 1 {
                    say!("  shell: {}", desktop.windows[0].describe());
                    // Wayland reports its outputs through a connection, and a
                    // connection here means a surface — so the monitors are not
                    // known until the first window exists, which is why
                    // placement is a second pass rather than part of opening
                    // one.
                    if outputs.is_empty() {
                        outputs = desktop.windows[0].outputs();
                    }
                }
            }
            Err(e) => {
                problem!("`{}` did not start: {e}", widget.name);
                refused.push(widget.name.clone());
            }
        }
    }
    if desktop.running.is_empty() {
        return Err(format!(
            "nothing started: {}. Every enabled widget in {} failed above.",
            refused.join(", "),
            config_path.display()
        ));
    }

    // Now that the monitors are known, put every widget where it belongs.
    // Stored relative to a named one, resolved against the ones that are
    // actually here: a widget whose monitor is unplugged appears on the primary
    // and keeps its name, so docking again puts it back.
    //
    // By name rather than by position in `enabled`: a widget that did not start
    // is not in `desktop.running`, and an index into one list used against the
    // other would place every later widget where its neighbour belongs.
    for index in 0..desktop.running.len() {
        let name = desktop.running[index].name.clone();
        let Some(entry) = enabled.iter().find(|w| w.name == name) else {
            continue;
        };
        // Not fatal either, and for the same reason: a widget that is on the
        // screen in the wrong place is better than a desktop that is not.
        if let Err(e) = place_widget(&mut host, &mut desktop, index, entry, &outputs, true) {
            problem!("could not place `{name}`: {e}");
        }
    }

    say!("  {} widget(s) running\n", desktop.windows.len());

    // What `uniwidged status` reads. From here on it is rewritten whenever the
    // answer changes; the guard removes it when this function returns, so a
    // desktop that stopped cleanly leaves nothing claiming to be running.
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let _state_file = crate::state::Written;
    record_state(&host, &desktop, &outputs, since);

    // Moving and resizing. The rule for what a press means lives in
    // `uniwidged-host`, tested there against every ambiguity; this is only the
    // wiring — X11 events in, window geometry and a saved placement out.
    let mut pointer = Interaction::new(1.0);
    pointer.set_bounds(desktop.windows.first().map(|w| w.desktop_bounds()));

    // A wall-clock limit, so an automated run terminates. A desktop has none.
    let until = std::env::var("UNIWIDGED_RUN_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|s| Instant::now() + Duration::from_secs(s));

    loop {
        if until.is_some_and(|deadline| Instant::now() > deadline) {
            say!("  exiting cleanly");
            return Ok(());
        }
        // Before anything else in the turn: a configuration that changed
        // should reach a widget before that widget is asked for its next view,
        // or the frame it is about to draw is the old one.
        if crate::daemon::take_pending() {
            if let Some(fresh) = reload(&mut host, config_path, &mut desktop, &outputs) {
                config = fresh;
            }
            record_state(&host, &desktop, &outputs, since);
            // A widget that arrived brought a window with it, and the pointer's
            // idea of the desktop's extent came from whichever window existed
            // first. On a desktop that had none, it had nothing to take it
            // from.
            if pointer.bounds().is_none() {
                pointer.set_bounds(desktop.windows.first().map(|w| w.desktop_bounds()));
            }
        }

        let timeout = host.next_timeout(Instant::now());
        // Bounded only so that `UNIWIDGED_RUN_SECS` can end an idle desktop;
        // without that this would block indefinitely, which is the whole point.
        let timeout = match (timeout, until) {
            (None, None) => None,
            (t, _) => Some(t.unwrap_or(Duration::from_millis(250))),
        };
        // Every window's connection, not the first one's. Each window carries
        // its own — there is no session object above them on either protocol —
        // so waiting on one is waiting on one widget, and a click on any other
        // sat unread until something else happened to wake the process. See
        // `linux::wait_any`. On Windows and macOS every window's messages
        // arrive on one thread's queue, and this is a wait on that.
        desktop::wait_any(&mut desktop.windows, timeout, Some(&wake)).map_err(|e| e.to_string())?;

        // Drained first, then acted on. A burst of motion events during a
        // drag is one geometry change rather than forty, and the last one is
        // the only one that describes where the pointer actually is.
        let mut events = Vec::new();
        for window in &mut desktop.windows {
            events.extend(window.drain_events().map_err(|e| e.to_string())?);
        }
        // A layer surface's size is the compositor's decision, not the
        // client's, and it may answer with less than was asked for on a small
        // output. That is not something the *user* did, so it does not arrive
        // as an input — but the swapchain has to follow it or the last frame is
        // stretched until something else redraws.
        for index in 0..desktop.running.len() {
            let window = desktop.running[index].window;
            if let Some((width, height)) = desktop.windows[window].take_resize() {
                host.renderer_mut()
                    .resize(desktop.running[index].surface, width, height)
                    .map_err(|e| e.to_string())?;
                host.redraw(desktop.running[index].id);
            }
        }
        let mut latest_motion = None;
        let mut monitors_changed = false;
        for input in events {
            match input {
                Input::Pressed {
                    window,
                    local,
                    root,
                } => {
                    let Some(index) = desktop.index_for_window(window) else {
                        continue;
                    };
                    let widget = &desktop.running[index];
                    let geometry = desktop.windows[widget.window].geometry();
                    // The host answers this, because the host did the layout:
                    // a press on a button belongs to the widget, and a press
                    // anywhere else moves it.
                    let on_interactive = host
                        .is_interactive_at(widget.id, Point::new(local.0 as f64, local.1 as f64));
                    match pointer.press(
                        widget.id,
                        root,
                        local,
                        geometry,
                        widget.locked,
                        on_interactive,
                    ) {
                        Press::ToWidget => {}
                        // Grabbed so that a drag faster than the compositor
                        // does not lose the pointer the moment it leaves the
                        // window.
                        Press::Move | Press::Resize(_) => {
                            let _ = desktop.windows[widget.window].grab_pointer();
                        }
                    }
                }
                Input::Motion { root, .. } => latest_motion = Some(root),
                Input::Released { root, .. } => {
                    if let Some(id) = pointer.widget()
                        && let Some(index) = desktop.index_of(id)
                    {
                        let _ = desktop.windows[desktop.running[index].window].release_pointer();
                    }
                    match pointer.release(root) {
                        Release::Click { widget, at } => {
                            match host.click(widget, Point::new(at.0 as f64, at.1 as f64)) {
                                Ok(Activated::Link { url, .. }) => open_link(&url),
                                Ok(Activated::LinkRefused { why, .. }) => {
                                    // Said rather than swallowed. A link that
                                    // does nothing when clicked is the worst
                                    // available outcome: its author cannot see
                                    // the grant is missing and its user cannot
                                    // see anything at all.
                                    problem!("that link was not opened — {why}");
                                }
                                Ok(_) => {}
                                Err(e) => problem!("{e}"),
                            }
                        }
                        Release::Placed { widget, rect } => {
                            if let Some(index) = desktop.index_of(widget) {
                                apply_geometry(&mut host, &mut desktop, index, rect)?;
                                save_placement(
                                    &mut config,
                                    config_path,
                                    &desktop.running[index].name,
                                    rect,
                                    &outputs,
                                );
                                record_state(&host, &desktop, &outputs, since);
                            }
                        }
                        Release::Nothing => {}
                    }
                }
                Input::Exposed { window } => {
                    // An X11 window has no backing store, so the contents are
                    // gone and the damage check would say nothing changed.
                    // Only the host knows how to put it back.
                    if let Some(index) = desktop.index_for_window(window) {
                        host.redraw(desktop.running[index].id);
                    }
                }
                // A monitor was plugged in, unplugged, moved or resized.
                // Collected rather than acted on here: every window's
                // connection reports it, and a desktop of six widgets would
                // otherwise re-resolve six times over one docking station.
                Input::OutputsChanged => monitors_changed = true,
                Input::Closed { .. } => {
                    say!("  closed");
                    return Ok(());
                }
            }
        }
        if monitors_changed {
            relocate(&mut host, &mut desktop, &config, &mut outputs, &mut pointer)?;
            record_state(&host, &desktop, &outputs, since);
        }

        if let Some(root) = latest_motion
            && let Some(rect) = pointer.motion(root)
            && let Some(id) = pointer.widget()
            && let Some(index) = desktop.index_of(id)
        {
            apply_geometry(&mut host, &mut desktop, index, rect)?;
        }

        let report = host
            .tick(Instant::now(), SystemTime::now())
            .map_err(|e| e.to_string())?;
        for line in host.take_log() {
            say!("  {line}");
        }
        for id in &report.suspended {
            problem!(
                "giving up on `{}` after repeated failures; its last \
                 frame stays on screen",
                host.widget(*id).map(|w| w.name.as_str()).unwrap_or("?")
            );
        }
        // A widget given up on is the single most useful thing `status` can
        // say, and the only way it changes without anybody touching the
        // configuration.
        if !report.suspended.is_empty() || !report.restarting.is_empty() {
            record_state(&host, &desktop, &outputs, since);
        }
    }
}

/// Writes what is on the screen where `uniwidged status` will find it.
///
/// Called whenever the answer changes — a widget started or stopped, a
/// placement moved, the monitors re-resolved, a widget given up on — and never
/// on a plain frame. That is the whole of its cost model: a desktop drawing a
/// clock every minute writes nothing here, and one where somebody is dragging a
/// widget writes once, on release, beside the configuration write that already
/// happens there.
///
/// It is built from the host and the window list together, because neither
/// knows the whole answer: the host knows what a widget *is* and whether its
/// supervisor has given up on it, and only this layer knows where its window
/// ended up and which monitor that is.
fn record_state(host: &Host, desktop: &Desktop, outputs: &[Output], since: i64) {
    use crate::state::{Health, State, WidgetState};
    use uniwidged_host::supervisor::State as Supervised;

    let widgets = desktop
        .running
        .iter()
        .map(|widget| {
            let rect = desktop.windows[widget.window].geometry();
            let (output, _) = placement::locate(rect, outputs);
            let (version, health) = match host.widget(widget.id) {
                Some(supervised) => (
                    supervised.manifest().version.clone(),
                    match supervised.state() {
                        Supervised::Running => Health::Running,
                        Supervised::Restarting => Health::Restarting,
                        // `Disabled` cannot be reached from here: a widget
                        // switched off in the file is stopped and removed, so
                        // it is not in this list to be described.
                        Supervised::Suspended(_) | Supervised::Disabled => Health::GivenUp,
                    },
                ),
                // Unreachable while one thread owns both lists, and answered
                // rather than skipped: a widget missing from `status` because
                // of a bookkeeping slip would look exactly like one that never
                // started.
                None => (String::new(), Health::GivenUp),
            };
            WidgetState {
                name: widget.name.clone(),
                version,
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                output,
                locked: widget.locked,
                health,
            }
        })
        .collect();
    State {
        pid: std::process::id() as i32,
        since,
        shell: desktop
            .windows
            .first()
            .map(|window| window.describe())
            .unwrap_or_default(),
        widgets,
    }
    .write();
}

/// Asks the session what the monitors are now, and puts every widget back where
/// its placement says.
///
/// What a laptop being docked looks like from inside the loop. Until this
/// existed, a placement was resolved at start-up and on a reload and at no
/// other time — so a widget stored against a monitor that was unplugged sat at
/// coordinates that had stopped existing, and one that came home when the
/// monitor returned did not.
///
/// **The list is asked for again rather than patched.** Both window systems
/// report *that* the layout changed — X11's screen-change notification carries
/// a screen size and nothing a placement can use, and Wayland announces and
/// withdraws outputs one at a time — so the only consistent view is the one
/// taken after the dust settles. It is asked of a window rather than of the
/// session, because that is the only way to ask on Wayland at all.
///
/// A session that answers with nothing is a session that cannot say — an X
/// server without RandR, a compositor with `wl_output` below version 4 — and
/// the right response is to leave every widget exactly where it is. Wiping the
/// list would move all of them to the origin.
fn relocate(
    host: &mut Host,
    desktop: &mut Desktop,
    config: &Config,
    outputs: &mut Vec<Output>,
    pointer: &mut Interaction,
) -> Result<(), String> {
    let Some(window) = desktop.windows.first() else {
        return Ok(());
    };
    let fresh = window.outputs();
    if fresh.is_empty() {
        return Ok(());
    }
    if fresh == *outputs {
        return Ok(());
    }
    say!("  the monitors changed:");
    for output in &fresh {
        say!(
            "    {} {}x{} at {},{}",
            output.name,
            output.rect.width,
            output.rect.height,
            output.rect.x,
            output.rect.y
        );
    }
    *outputs = fresh;
    // The extent a widget may be dragged within, which has just changed by
    // definition.
    pointer.set_bounds(desktop.windows.first().map(|w| w.desktop_bounds()));

    for index in 0..desktop.running.len() {
        let Some(entry) = config.widget(&desktop.running[index].name).cloned() else {
            continue;
        };
        if let Err(e) = place_widget(host, desktop, index, &entry, outputs, false) {
            problem!("could not place `{}`: {e}", entry.name);
        }
    }
    Ok(())
}

/// Moves the window, and the surface with it.
///
/// The two have to happen together. A window that changed size has a swapchain
/// that has not, and presenting into it stretches the last frame until
/// something resizes and redraws — which is why the redraw is not left to the
/// damage check: the widget's *view* has not changed, only the box it is drawn
/// in, and the damage check compares views.
fn apply_geometry(
    host: &mut Host,
    desktop: &mut Desktop,
    index: usize,
    rect: PhysicalRect,
) -> Result<(), String> {
    let widget = &desktop.running[index];
    let before = desktop.windows[widget.window].geometry();
    desktop.windows[widget.window]
        .set_geometry(rect)
        .map_err(|e| e.to_string())?;
    if (before.width, before.height) != (rect.width, rect.height) {
        host.renderer_mut()
            .resize(widget.surface, rect.width, rect.height)
            .map_err(|e| e.to_string())?;
        host.redraw(widget.id);
    }
    Ok(())
}

/// Writes a widget's new placement back to the configuration.
///
/// On release rather than on every motion: a drag is a hundred positions and
/// one decision, and writing the file a hundred times would be a hundred
/// chances to be interrupted half way through it.
///
/// A failure here is reported and not fatal. The widget is where the user put
/// it; what has been lost is that it will still be there tomorrow, and
/// stopping the desktop over that would be worse.
fn save_placement(
    config: &mut Config,
    path: &Path,
    name: &str,
    rect: PhysicalRect,
    outputs: &[Output],
) {
    let Some(widget) = config.widget_mut(name) else {
        return;
    };
    // Recorded against the monitor showing most of the widget, and relative to
    // it — so that the same file puts the widget back in the same place on a
    // desktop whose layout has changed since.
    let (output, relative) = placement::locate(rect, outputs);
    widget.placement.output = output;
    widget.placement.x = relative.x;
    widget.placement.y = relative.y;
    widget.placement.width = relative.width;
    widget.placement.height = relative.height;
    if let Err(e) = config.write(path) {
        problem!("could not save where `{name}` was put: {e}");
    }
}
