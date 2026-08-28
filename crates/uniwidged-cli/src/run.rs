//! The desktop, running on X11.
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
use uniwidged_shell::platform::linux::{self, DesktopWindow, Session};
use uniwidged_shell::platform::x11::{AlphaPath, X11Window};
use uniwidged_shell::{Input, Output, WakePipe};

/// One widget on the desktop: what it is, where it draws, and whether the user
/// has pinned it down.
struct Running {
    id: WidgetId,
    name: String,
    /// Index into the window list, which owns the windows for the process's
    /// lifetime — dropping one destroys the surface wgpu is drawing into.
    window: usize,
    surface: SurfaceId,
    locked: bool,
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
    // No shell. `xdg-open` is executed directly with the URL as one argument,
    // so nothing in it can become a second command — the same rule the
    // `command` capability follows, and the reason both are worth stating.
    match std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_child) => println!("  opening {url}"),
        Err(e) => eprintln!("uniwidged: could not open {url}: {e}"),
    }
}

/// Reads the configuration again and applies what can be applied.
///
/// What a `SIGHUP` does. The split is the one `ARCHITECTURE.md` §5 makes about
/// surfaces: [`uniwidged_host::reload::apply`] settles what the *widgets* do —
/// which is settings, because a setting is a value behind a shared handle —
/// and this settles what the *windows* do, because only this layer owns them.
///
/// Everything is best effort and nothing here can stop the desktop. A
/// configuration that has become unreadable is a reason to keep running with
/// the one already loaded, not a reason to take somebody's widgets off their
/// screen.
fn reload(
    host: &mut Host,
    config_path: &Path,
    running: &mut [Running],
    windows: &mut [DesktopWindow],
    outputs: &[Output],
) {
    let config = match Config::read(config_path) {
        Ok(config) => config,
        Err(e) => {
            eprintln!(
                "uniwidged: {} is not readable, so nothing changed: {e}",
                config_path.display()
            );
            return;
        }
    };

    let applied = uniwidged_host::reload::apply(host, &config);
    for complaint in &applied.complaints {
        eprintln!("uniwidged: {complaint}");
    }
    for id in &applied.reconfigured {
        // Named, because "the setting took effect" is the fact somebody
        // changing one is waiting for — and because a widget that answered
        // that its view is unchanged still had its configuration applied.
        if let Some(widget) = host.widget(*id) {
            println!("  `{}` reconfigured", widget.name);
        }
    }
    for name in &applied.added {
        println!("  `{name}` is new; it starts when the desktop does");
    }
    for name in &applied.removed {
        println!("  `{name}` is gone from the configuration; it stops when the desktop does");
    }

    // Placement, which the host cannot do: it does not own windows. Counted,
    // because "nothing changed" is a line about the *whole* reload and the
    // host's half cannot see this one — a live run printed it over a window it
    // had just moved.
    let mut moved = 0;
    for entry in &running[..] {
        let Some(widget) = config.widget(&entry.name) else {
            continue;
        };
        let resolved = placement::resolve(&widget.placement, outputs);
        let window = &mut windows[entry.window];
        if resolved.rect != window.geometry() {
            if let Err(e) = window.set_geometry(resolved.rect) {
                eprintln!("uniwidged: could not move `{}`: {e}", entry.name);
                continue;
            }
            println!(
                "  `{}` moved to {},{} {}x{}",
                entry.name,
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
    for entry in running.iter_mut() {
        if let Some(widget) = config.widget(&entry.name)
            && entry.locked != widget.placement.locked
        {
            entry.locked = widget.placement.locked;
            println!(
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

    if applied.is_quiet() && moved == 0 && relocked == 0 {
        println!("  configuration read again; nothing changed");
    }
}

/// What `uniwidged doctor` can say about this session without starting one.
pub fn doctor() {
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
    // What a widget author needs when their widget reports `unavailable` and
    // they are trying to work out whether that is their manifest, their user's
    // choice, or this host.
    match uniwidged_host::Host::new(
        uniwidged_compose::Theme::default(),
        Duration::from_millis(16),
    ) {
        Ok(mut host) => {
            host.set_data_root(crate::paths::data_root());
            host.set_keyring(std::sync::Arc::new(uniwidged_runtime::FileKeyring::new(
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
    let _pid_file = match crate::daemon::listen(std::os::fd::AsRawFd::as_raw_fd(&wake.as_fd())) {
        Ok(guard) => {
            println!("  pid file: {}", guard.path().display());
            Some(guard)
        }
        Err(e) => {
            eprintln!(
                "uniwidged: no pid file, so `uniwidged reload` will not find this desktop: {e}"
            );
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
    host.set_keyring(std::sync::Arc::new(uniwidged_runtime::FileKeyring::new(
        crate::paths::keyring_dir(),
    )));
    // Every widget's window, kept alive for as long as the desktop is: dropping
    // one destroys the window and, with it, the surface wgpu is drawing into.
    // X11 can be asked before a window exists; Wayland reports its outputs
    // through a connection, and a connection here means a surface. So widgets
    // are opened at the coordinates they were saved with and *then* placed —
    // which is one extra `set_geometry` on X11 and the only possible order on
    // Wayland.
    let mut outputs: Vec<Output> = linux::outputs();

    let mut windows: Vec<DesktopWindow> = Vec::new();
    let mut running: Vec<Running> = Vec::new();
    let mut by_window: HashMap<u32, usize> = HashMap::new();

    for widget in &enabled {
        let wasm = std::fs::read(&widget.component)
            .map_err(|e| format!("{}: {e}", widget.component.display()))?;
        let manifest_text = std::fs::read_to_string(widget.manifest_path())
            .map_err(|e| format!("{}: {e}", widget.manifest_path().display()))?;
        let manifest = Manifest::parse(&manifest_text).map_err(|e| e.to_string())?;
        let policy = widget.policy(&manifest).map_err(|e| e.to_string())?;

        let placement = &widget.placement;
        let geometry =
            PhysicalRect::new(placement.x, placement.y, placement.width, placement.height);
        let window = DesktopWindow::open(geometry, &widget.name).map_err(|e| e.to_string())?;
        if windows.is_empty() {
            println!("  shell: {}", window.describe());
            if outputs.is_empty() {
                outputs = window.outputs();
            }
        }
        if !window.has_compositor() {
            eprintln!(
                "uniwidged: no compositing manager is running, so `{}` will be \
                 an opaque rectangle rather than a widget",
                widget.name
            );
        }

        // The user's own values, resolved against this manifest before the
        // widget is asked for a view. `install` starts every widget at its
        // declared defaults, so without this a configuration would be a file
        // nothing reads.
        let (values, complaints) = widget.settings(&manifest);
        let id = host
            .install(&wasm, manifest, policy)
            .map_err(|e| e.to_string())?;
        if let Err(e) = host.configure(id, &values) {
            eprintln!("uniwidged: could not configure `{}`: {e}", widget.name);
        }
        for complaint in complaints {
            // A line in somebody's configuration that is being ignored has to
            // be findable. `uniwidged configure <name>` says the same thing
            // without starting a desktop.
            eprintln!("uniwidged: `{}`: {complaint}", widget.name);
        }
        let (display, handle) = window.raw_handles();
        // SAFETY: the window is kept in `windows` for the lifetime of the
        // host, and was configured and mapped above.
        let surface = unsafe {
            host.renderer_mut().add_swapchain_surface(
                display,
                handle,
                geometry.width,
                geometry.height,
            )
        }
        .map_err(|e| e.to_string())?;
        host.attach(id, surface).map_err(|e| e.to_string())?;

        if host.renderer_mut().alpha_mode(surface)
            == Some(uniwidged_render::wgpu::CompositeAlphaMode::Opaque)
        {
            eprintln!(
                "uniwidged: this driver offers no transparent swapchain, so `{}` \
                 will sit on a solid rectangle — see ARCHITECTURE.md §10.1",
                widget.name
            );
        }

        by_window.insert(window.window_id(), running.len());
        running.push(Running {
            id,
            name: widget.name.clone(),
            window: windows.len(),
            surface,
            locked: placement.locked,
        });
        windows.push(window);
    }

    // Now that the monitors are known, put every widget where it belongs.
    // Stored relative to a named one, resolved against the ones that are
    // actually here: a widget whose monitor is unplugged appears on the primary
    // and keeps its name, so docking again puts it back.
    for (index, widget) in enabled.iter().enumerate() {
        let resolved = placement::resolve(&widget.placement, &outputs);
        if resolved.displaced {
            eprintln!(
                "uniwidged: `{}` belongs on {}, which is not connected; it is \
                 on {} for now and will go back when that monitor returns",
                widget.name,
                widget
                    .placement
                    .output
                    .as_deref()
                    .unwrap_or("another monitor"),
                resolved.output.as_deref().unwrap_or("this one"),
            );
        }
        if resolved.rect != windows[running[index].window].geometry() {
            windows[running[index].window]
                .set_geometry(resolved.rect)
                .map_err(|e| e.to_string())?;
        }
        let at = windows[running[index].window].geometry();
        println!(
            "  {} at {},{} {}x{}{}",
            widget.name,
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

    println!("  {} widget(s) running\n", windows.len());

    // Moving and resizing. The rule for what a press means lives in
    // `uniwidged-host`, tested there against every ambiguity; this is only the
    // wiring — X11 events in, window geometry and a saved placement out.
    let mut pointer = Interaction::new(1.0);
    pointer.set_bounds(Some(windows[0].desktop_bounds()));

    // A wall-clock limit, so an automated run terminates. A desktop has none.
    let until = std::env::var("UNIWIDGED_RUN_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|s| Instant::now() + Duration::from_secs(s));

    loop {
        if until.is_some_and(|deadline| Instant::now() > deadline) {
            println!("  exiting cleanly");
            return Ok(());
        }
        // Before anything else in the turn: a configuration that changed
        // should reach a widget before that widget is asked for its next view,
        // or the frame it is about to draw is the old one.
        if crate::daemon::take_pending() {
            reload(&mut host, config_path, &mut running, &mut windows, &outputs);
        }

        let timeout = host.next_timeout(Instant::now());
        // Bounded only so that `UNIWIDGED_RUN_SECS` can end an idle desktop;
        // without that this would block indefinitely, which is the whole point.
        let timeout = match (timeout, until) {
            (None, None) => None,
            (t, _) => Some(t.unwrap_or(Duration::from_millis(250))),
        };
        windows[0]
            .wait_with(timeout, Some(&wake))
            .map_err(|e| e.to_string())?;

        // Drained first, then acted on. A burst of motion events during a
        // drag is one geometry change rather than forty, and the last one is
        // the only one that describes where the pointer actually is.
        let mut events = Vec::new();
        for window in &mut windows {
            events.extend(window.drain_events().map_err(|e| e.to_string())?);
        }
        // A layer surface's size is the compositor's decision, not the
        // client's, and it may answer with less than was asked for on a small
        // output. That is not something the *user* did, so it does not arrive
        // as an input — but the swapchain has to follow it or the last frame is
        // stretched until something else redraws.
        for index in 0..running.len() {
            if let Some((width, height)) = windows[running[index].window].take_resize() {
                host.renderer_mut()
                    .resize(running[index].surface, width, height)
                    .map_err(|e| e.to_string())?;
                host.redraw(running[index].id);
            }
        }
        let mut latest_motion = None;
        for input in events {
            match input {
                Input::Pressed {
                    window,
                    local,
                    root,
                } => {
                    let Some(&index) = by_window.get(&window) else {
                        continue;
                    };
                    let widget = &running[index];
                    let geometry = windows[widget.window].geometry();
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
                            let _ = windows[widget.window].grab_pointer();
                        }
                    }
                }
                Input::Motion { root, .. } => latest_motion = Some(root),
                Input::Released { root, .. } => {
                    if let Some(id) = pointer.widget()
                        && let Some(index) = running.iter().position(|w| w.id == id)
                    {
                        let _ = windows[running[index].window].release_pointer();
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
                                    eprintln!("uniwidged: that link was not opened — {why}");
                                }
                                Ok(_) => {}
                                Err(e) => eprintln!("uniwidged: {e}"),
                            }
                        }
                        Release::Placed { widget, rect } => {
                            if let Some(index) = running.iter().position(|w| w.id == widget) {
                                apply_geometry(&mut host, &mut windows, &running[index], rect)?;
                                save_placement(
                                    &mut config,
                                    config_path,
                                    &running[index].name,
                                    rect,
                                    &outputs,
                                );
                            }
                        }
                        Release::Nothing => {}
                    }
                }
                Input::Exposed { window } => {
                    // An X11 window has no backing store, so the contents are
                    // gone and the damage check would say nothing changed.
                    // Only the host knows how to put it back.
                    if let Some(&index) = by_window.get(&window) {
                        host.redraw(running[index].id);
                    }
                }
                Input::Closed { .. } => {
                    println!("  closed");
                    return Ok(());
                }
            }
        }
        if let Some(root) = latest_motion
            && let Some(rect) = pointer.motion(root)
            && let Some(id) = pointer.widget()
            && let Some(index) = running.iter().position(|w| w.id == id)
        {
            apply_geometry(&mut host, &mut windows, &running[index], rect)?;
        }

        let report = host
            .tick(Instant::now(), SystemTime::now())
            .map_err(|e| e.to_string())?;
        for line in host.take_log() {
            println!("  {line}");
        }
        for id in &report.suspended {
            eprintln!(
                "uniwidged: giving up on `{}` after repeated failures; its last \
                 frame stays on screen",
                host.widget(*id).map(|w| w.name.as_str()).unwrap_or("?")
            );
        }
    }
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
    windows: &mut [DesktopWindow],
    widget: &Running,
    rect: PhysicalRect,
) -> Result<(), String> {
    let before = windows[widget.window].geometry();
    windows[widget.window]
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
        eprintln!("uniwidged: could not save where `{name}` was put: {e}");
    }
}
