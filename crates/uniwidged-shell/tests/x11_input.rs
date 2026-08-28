//! A drag, measured.
//!
//! `uniwidged-host`'s interaction tests settle what a press *means*; nothing in
//! them touches a window. This is the other half: a real X server, a real
//! desktop-layer window, synthetic pointer input through `XTest`, and the
//! window's geometry read back from the server afterwards.
//!
//! It is worth the machinery because the failure modes here are not the ones a
//! unit test finds. A drag computed from window-relative coordinates *passes*
//! every arithmetic test and shivers on screen. A missing pointer grab works
//! perfectly until the pointer leaves the window. Neither is visible without a
//! server.
//!
//! The server is started by the test rather than assumed, so this needs no
//! session and no display. Without `Xvfb` it skips, and
//! `UNIWIDGED_REQUIRE_X11=1` turns that into a failure — the same bargain the
//! golden images make with `UNIWIDGED_REQUIRE_GPU`, and CI sets both.

#![cfg(all(unix, not(target_os = "macos")))]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use uniwidged_core::WidgetId;
use uniwidged_core::geom::PhysicalRect;
use uniwidged_host::interaction::{Interaction, Press, Release};
use uniwidged_shell::Input;
use uniwidged_shell::platform::x11::{WindowKind, X11Window};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::xcb_ffi::XCBConnection;

const WIDGET: WidgetId = WidgetId(1);
const START: PhysicalRect = PhysicalRect::new(48, 48, 320, 200);

/// A nested X server, stopped when the test ends.
struct Server {
    display: String,
    number: u32,
    child: Child,
    /// A connection held open for as long as the server is wanted.
    ///
    /// Not decoration. An X server **resets** when its last client
    /// disconnects, and a connection arriving during that reset is refused —
    /// which showed up here as the probe connection succeeding and the very
    /// next one failing with a bare `UnknownError`. Keeping one client
    /// attached means the server never has zero.
    _keepalive: XCBConnection,
}

impl Server {
    /// `None` when there is no `Xvfb` to start.
    fn start() -> Option<Self> {
        // Tests in one binary run in parallel, and the first version of this
        // looked for an unused socket — which two threads do at the same time,
        // agree on, and then both start a server on. The symptom was one test
        // seeing another's pointer, and it took the failure to notice: an
        // assertion about a resize reported a *move*, by exactly the distance
        // a different test had dragged.
        //
        // A counter is the fix. The socket check stays, for servers this
        // process did not start.
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(90);
        for _ in 0..30 {
            let number = NEXT.fetch_add(1, Ordering::Relaxed);
            let display = format!(":{number}");
            if std::path::Path::new(&format!("/tmp/.X11-unix/X{number}")).exists() {
                continue;
            }
            let child = Command::new("Xvfb")
                .args([
                    &display,
                    "-screen",
                    "0",
                    "1280x720x24",
                    // The 32-bit visual a widget needs comes from RENDER, and
                    // Composite is what makes it mean anything. Same flags as
                    // the M4 spike, for the same reasons.
                    "+extension",
                    "RENDER",
                    "+extension",
                    "Composite",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            // The socket appears before the server is ready to accept, so the
            // wait is on a successful connection rather than on the file.
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut child = child;
            while Instant::now() < deadline {
                if let Ok((conn, _)) = XCBConnection::connect(Some(
                    &std::ffi::CString::new(display.clone()).expect("a display name"),
                )) {
                    return Some(Server {
                        display,
                        number,
                        child,
                        _keepalive: conn,
                    });
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            // That number did not come up. Another may.
            let _ = child.kill();
            let _ = child.wait();
            continue;
        }
        None
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // A killed server leaves its socket and lock file behind, and the next
        // run of this binary then walks past every number it has ever used.
        // Nothing else owns these: the number came from this process.
        let _ = std::fs::remove_file(format!("/tmp/.X11-unix/X{}", self.number));
        let _ = std::fs::remove_file(format!("/tmp/.X{}-lock", self.number));
    }
}

/// Starts a server, or says why the test is not running.
///
/// Returning `None` skips. `UNIWIDGED_REQUIRE_X11=1` makes that a failure
/// instead, so a CI runner missing `Xvfb` is loud rather than green.
fn server() -> Option<Server> {
    match Server::start() {
        Some(server) => Some(server),
        None => {
            assert!(
                std::env::var("UNIWIDGED_REQUIRE_X11").is_err(),
                "UNIWIDGED_REQUIRE_X11 is set and no Xvfb could be started"
            );
            eprintln!("skipping: no Xvfb. Set UNIWIDGED_REQUIRE_X11=1 to make this a failure.");
            None
        }
    }
}

/// The second connection, for synthesising input.
///
/// Deliberately not the window's own: `XTest` requests and window events on one
/// connection would let the test's requests be answered out of order with the
/// events it is waiting for, and the resulting flakiness would look like a bug
/// in the code under test.
struct Pointer {
    conn: XCBConnection,
    root: xproto::Window,
}

impl Pointer {
    fn open(display: &str) -> Self {
        let (conn, screen) = XCBConnection::connect(Some(
            &std::ffi::CString::new(display.to_owned()).expect("a display name"),
        ))
        .expect("a second connection");
        let root = conn.setup().roots[screen].root;
        Self { conn, root }
    }

    fn move_to(&self, x: i16, y: i16) {
        self.conn
            .xtest_fake_input(
                xproto::MOTION_NOTIFY_EVENT,
                // Absolute coordinates, not a relative warp.
                0,
                0,
                self.root,
                x,
                y,
                0,
            )
            .expect("fake motion");
        self.conn.flush().expect("flush");
    }

    fn button(&self, down: bool) {
        let event = if down {
            xproto::BUTTON_PRESS_EVENT
        } else {
            xproto::BUTTON_RELEASE_EVENT
        };
        self.conn
            .xtest_fake_input(event, 1, 0, self.root, 0, 0, 0)
            .expect("fake button");
        self.conn.flush().expect("flush");
    }
}

/// A widget window and the pointer state that moves it.
struct Desktop {
    window: X11Window,
    pointer: Interaction,
    locked: bool,
    /// Set when the release was a click rather than a placement.
    clicked: Option<(i32, i32)>,
    placed: Option<PhysicalRect>,
    /// How many button releases have been *processed*, whatever they turned
    /// out to mean. What a gesture's end waits on: a release that produced
    /// neither a click nor a placement is a real answer, and one this file has
    /// three tests about.
    releases: u32,
}

impl Desktop {
    fn open(display: &str, locked: bool) -> Self {
        let window =
            X11Window::open_on(Some(display), START, "drag test", WindowKind::DesktopWidget)
                .expect("a desktop-layer window");
        let mut pointer = Interaction::new(1.0);
        pointer.set_bounds(Some(window.desktop_bounds()));
        Self {
            window,
            pointer,
            locked,
            clicked: None,
            placed: None,
            releases: 0,
        }
    }

    /// Pumps events for a while, doing what the desktop's own loop does.
    /// Pumps until `done` or until `limit` has passed.
    ///
    /// For the end of a gesture, where what the test is waiting for is a fact
    /// rather than a duration — see the note in `drag`.
    fn pump_until(&mut self, limit: Duration, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline && !done(self) {
            self.pump(Duration::from_millis(20));
        }
    }

    fn pump(&mut self, how_long: Duration) {
        let deadline = Instant::now() + how_long;
        while Instant::now() < deadline {
            let _ = self.window.wait(Some(Duration::from_millis(20)));
            let events = self.window.drain_events().expect("events");
            let mut latest_motion = None;
            for input in events {
                match input {
                    Input::Pressed { local, root, .. } => {
                        let geometry = self.window.geometry();
                        // Nothing interactive: this test is about the frame,
                        // and what a button does to the rule is settled in
                        // `uniwidged-host`.
                        if let Press::Move | Press::Resize(_) =
                            self.pointer
                                .press(WIDGET, root, local, geometry, self.locked, false)
                        {
                            let _ = self.window.grab_pointer();
                        }
                    }
                    Input::Motion { root, .. } => latest_motion = Some(root),
                    Input::Released { root, .. } => {
                        self.releases += 1;
                        let _ = self.window.release_pointer();
                        match self.pointer.release(root) {
                            Release::Click { at, .. } => self.clicked = Some(at),
                            Release::Placed { rect, .. } => {
                                self.placed = Some(rect);
                                let _ = self.window.set_geometry(rect);
                            }
                            Release::Nothing => {}
                        }
                    }
                    _ => {}
                }
            }
            if let Some(root) = latest_motion
                && let Some(rect) = self.pointer.motion(root)
            {
                self.window.set_geometry(rect).expect("set geometry");
            }
        }
    }

    /// Where the *server* thinks the window is — not where this process
    /// believes it put it, which is the whole point of asking.
    fn geometry_from_server(&self, pointer: &Pointer) -> PhysicalRect {
        let reply = pointer
            .conn
            .get_geometry(self.window.window_id())
            .expect("geometry request")
            .reply()
            .expect("geometry");
        // A child of the root with no window manager: `x`/`y` are already
        // desktop coordinates.
        PhysicalRect::new(
            reply.x as i32,
            reply.y as i32,
            reply.width as u32,
            reply.height as u32,
        )
    }
}

/// Presses at `from`, travels to `to` in steps, releases.
///
/// In steps rather than one jump, because that is what a hand does — and
/// because a single motion event would not exercise the coalescing the desktop
/// loop does with a burst of them.
fn drag(pointer: &Pointer, desktop: &mut Desktop, from: (i16, i16), to: (i16, i16)) {
    pointer.move_to(from.0, from.1);
    desktop.pump(Duration::from_millis(80));
    pointer.button(true);
    desktop.pump(Duration::from_millis(80));

    const STEPS: i16 = 8;
    for step in 1..=STEPS {
        let x = from.0 + (to.0 - from.0) * step / STEPS;
        let y = from.1 + (to.1 - from.1) * step / STEPS;
        pointer.move_to(x, y);
        desktop.pump(Duration::from_millis(30));
    }
    pointer.button(false);
    // Waited for rather than slept through. A fixed window is a bet on how
    // busy the machine is, and it is a bet this file lost once: the release
    // arrived after the pump had ended, the drag looked like it had never
    // happened, and the failure said `placed: None` with nothing to suggest
    // why. The generous ceiling is only reached when something is genuinely
    // wrong, and then the assertion that follows says what.
    let before = desktop.releases;
    desktop.pump_until(Duration::from_secs(5), |desktop| desktop.releases > before);
}

#[test]
fn a_widget_can_be_dragged_and_the_server_agrees_where_it_went() {
    let Some(server) = server() else { return };
    let pointer = Pointer::open(&server.display);
    let mut desktop = Desktop::open(&server.display, false);
    desktop.pump(Duration::from_millis(150));

    // The middle of the widget: not on any resize edge.
    drag(&pointer, &mut desktop, (208, 148), (408, 348));

    let placed = desktop
        .placed
        .expect("the drag should have placed the widget");
    assert_eq!(
        placed,
        PhysicalRect::new(248, 248, 320, 200),
        "a 200-pixel drag moves the widget 200 pixels"
    );
    assert_eq!(
        desktop.geometry_from_server(&pointer),
        placed,
        "and the X server agrees, which is the half no unit test can check"
    );
    assert!(desktop.clicked.is_none(), "a drag is not also a click");
}

#[test]
fn a_press_that_does_not_travel_is_a_click_and_moves_nothing() {
    let Some(server) = server() else { return };
    let pointer = Pointer::open(&server.display);
    let mut desktop = Desktop::open(&server.display, false);
    desktop.pump(Duration::from_millis(150));

    pointer.move_to(208, 148);
    desktop.pump(Duration::from_millis(80));
    pointer.button(true);
    desktop.pump(Duration::from_millis(80));
    pointer.button(false);
    desktop.pump(Duration::from_millis(120));

    assert_eq!(
        desktop.clicked,
        Some((160, 100)),
        "the click is reported in the window's own coordinates"
    );
    assert!(desktop.placed.is_none());
    assert_eq!(desktop.geometry_from_server(&pointer), START);
}

#[test]
fn a_corner_resizes_rather_than_moves() {
    let Some(server) = server() else { return };
    let pointer = Pointer::open(&server.display);
    let mut desktop = Desktop::open(&server.display, false);
    desktop.pump(Duration::from_millis(150));

    // The bottom-right corner, in desktop coordinates: 48+320-2, 48+200-2.
    drag(&pointer, &mut desktop, (366, 246), (466, 346));

    let placed = desktop
        .placed
        .expect("the drag should have placed the widget");
    assert_eq!(
        placed,
        PhysicalRect::new(48, 48, 420, 300),
        "the corner grew the widget and left its origin alone"
    );
    assert_eq!(desktop.geometry_from_server(&pointer), placed);
}

#[test]
fn a_locked_widget_does_not_move() {
    let Some(server) = server() else { return };
    let pointer = Pointer::open(&server.display);
    let mut desktop = Desktop::open(&server.display, true);
    desktop.pump(Duration::from_millis(150));

    drag(&pointer, &mut desktop, (208, 148), (408, 348));

    assert!(desktop.placed.is_none(), "locked means locked");
    assert_eq!(desktop.geometry_from_server(&pointer), START);
}

#[test]
fn a_display_reports_its_monitors_by_name() {
    // Placement stores an output *name*, and this is the half that has to
    // supply one. A nested server has exactly one monitor covering the whole
    // screen, which is enough to check that the RandR path works, that the
    // name is a name rather than an atom number, and that the geometry is the
    // screen's rather than the window's.
    let Some(server) = server() else { return };
    let outputs = uniwidged_shell::platform::x11::outputs(Some(&server.display))
        .expect("this server has RandR");

    assert_eq!(outputs.len(), 1, "one screen, one monitor: {outputs:?}");
    let only = &outputs[0];
    assert!(
        !only.name.is_empty() && only.name.is_ascii(),
        "a monitor name should be readable, got {:?}",
        only.name
    );
    assert_eq!(only.rect, PhysicalRect::new(0, 0, 1280, 720));
    // Deliberately *not* asserted to be primary: Xvfb reports its only monitor
    // without the flag, which is why `placement::primary` falls back to the
    // first output rather than trusting that one is marked. A real desktop
    // marks one; a nested server is a reminder that not every desktop does.
    println!("  {} primary={}", only.name, only.primary);
}

#[test]
fn a_window_and_the_display_agree_about_the_monitors() {
    // The two paths — with a window and without — must not drift. The one
    // without exists because placement is decided before any window is created.
    let Some(server) = server() else { return };
    let desktop = Desktop::open(&server.display, false);
    assert_eq!(
        desktop
            .window
            .outputs()
            .expect("outputs through the window"),
        uniwidged_shell::platform::x11::outputs(Some(&server.display)).expect("outputs directly")
    );
}

#[test]
fn a_widget_dragged_across_the_desktop_is_recorded_against_the_monitor_it_lands_on() {
    // The two halves joined: a real drag, then the placement that would be
    // written to `config.toml`. Resolving that placement again has to give
    // back the rectangle the window is actually at — which is the property
    // that makes a saved position mean anything.
    use uniwidged_host::config::Placement;
    use uniwidged_host::placement;

    let Some(server) = server() else { return };
    let pointer = Pointer::open(&server.display);
    let mut desktop = Desktop::open(&server.display, false);
    let outputs = desktop.window.outputs().expect("outputs");
    desktop.pump(Duration::from_millis(150));

    drag(&pointer, &mut desktop, (208, 148), (608, 448));
    let placed = desktop.placed.expect("placed");

    let (output, relative) = placement::locate(placed, &outputs);
    assert!(output.is_some(), "it landed on the only monitor there is");
    let stored = Placement {
        output,
        x: relative.x,
        y: relative.y,
        width: relative.width,
        height: relative.height,
        locked: false,
    };
    assert_eq!(
        placement::resolve(&stored, &outputs).rect,
        placed,
        "what was saved should put the widget back where it is"
    );
    assert_eq!(desktop.geometry_from_server(&pointer), placed);
}

// ---------------------------------------------------------------------------
// Two monitors
// ---------------------------------------------------------------------------

/// Defines a virtual RandR monitor on a running server.
///
/// `xrandr --setmonitor` is the whole reason this is testable at all. A nested
/// `Xvfb` has one screen and no real outputs, but RandR 1.5 lets a *client*
/// declare monitors over a screen's geometry — which is exactly what
/// `randr_get_monitors` then reports, and exactly what `uniwidged-shell`
/// reads. So the thing under test sees two named monitors side by side,
/// through the same protocol path a docked laptop uses.
///
/// This is the gap `ROADMAP.md` named for two milestones: the placement logic
/// was tested against a docked-laptop layout in memory, and the enumeration
/// against a server with one screen. Nobody had watched the two meet.
fn set_monitor(display: &str, name: &str, geometry: &str) -> bool {
    Command::new("xrandr")
        .env("DISPLAY", display)
        .args(["--setmonitor", name, geometry, "none"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn delete_monitor(display: &str, name: &str) -> bool {
    Command::new("xrandr")
        .env("DISPLAY", display)
        .args(["--delmonitor", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Two monitors, or a skip saying why.
///
/// `xrandr` is a separate binary and a separate package from `Xvfb`, so its
/// absence is treated the way `Xvfb`'s is: a skip, unless the run has said it
/// requires X11, in which case it is a failure. A green run that silently
/// checked nothing is the thing this repository keeps refusing.
fn two_monitors(display: &str) -> bool {
    // The left one is deliberately *not* at the origin in one of the two
    // layouts below, because an output whose corner is (0, 0) makes an
    // arithmetic error in either direction invisible.
    let ok = set_monitor(display, "LEFT-1", "640/170x720/240+0+0")
        && set_monitor(display, "RIGHT-1", "640/170x720/240+640+0");
    if !ok {
        assert!(
            std::env::var("UNIWIDGED_REQUIRE_X11").is_err(),
            "UNIWIDGED_REQUIRE_X11 is set and `xrandr --setmonitor` did not work"
        );
        eprintln!(
            "skipping: no usable xrandr. Set UNIWIDGED_REQUIRE_X11=1 to make this a failure."
        );
    }
    ok
}

#[test]
fn two_monitors_are_enumerated_by_name_and_by_where_they_are() {
    // The property every placement depends on: a monitor is reported by a name
    // that does not renumber, with the rectangle it occupies in the desktop's
    // coordinate space. An index would be enough for one screen and wrong for
    // two.
    let Some(server) = server() else { return };
    if !two_monitors(&server.display) {
        return;
    }

    let outputs = uniwidged_shell::platform::x11::outputs(Some(&server.display))
        .expect("a server with monitors should enumerate them");
    let by_name = |name: &str| {
        outputs
            .iter()
            .find(|output| output.name == name)
            .unwrap_or_else(|| panic!("no output named {name} in {outputs:?}"))
    };
    assert_eq!(by_name("LEFT-1").rect.x, 0);
    assert_eq!(by_name("LEFT-1").rect.width, 640);
    assert_eq!(by_name("RIGHT-1").rect.x, 640);
    assert_eq!(by_name("RIGHT-1").rect.width, 640);

    // A finding worth keeping, because it is not what the code was written
    // against: **no monitor here is flagged primary**. X reports one only when
    // something has set it, and a nested server with no real outputs never
    // does — as, in the field, do a fresh RandR configuration and more than
    // one Wayland compositor. `placement::primary` already falls back to the
    // first output rather than indexing into nothing, and this is the run that
    // exercises that path rather than the one everybody assumes.
    assert!(
        outputs.iter().filter(|output| output.primary).count() <= 1,
        "two primaries would make the fallback depend on iteration order: {outputs:?}"
    );

    // And the automatic whole-screen monitor X keeps alongside the declared
    // ones. Not tidied away, because a real desktop can show one too — a
    // laptop whose outputs are all disconnected — and a placement has to
    // survive being resolved against a list that holds it.
    assert!(
        outputs.iter().any(|output| output.rect.width == 1280),
        "the automatic monitor should still be listed: {outputs:?}"
    );
}

#[test]
fn a_widget_on_the_second_monitor_opens_where_the_second_monitor_is() {
    // The end-to-end claim: a placement stored *relative to a named output*
    // becomes a window at the right desktop coordinates, and the server agrees.
    // The arithmetic is `output.x + stored.x`, and a sign error in it is
    // invisible on a single screen whose origin is zero.
    let Some(server) = server() else { return };
    if !two_monitors(&server.display) {
        return;
    }
    let outputs = uniwidged_shell::platform::x11::outputs(Some(&server.display)).expect("outputs");

    let stored = uniwidged_host::config::Placement {
        output: Some("RIGHT-1".into()),
        x: 32,
        y: 24,
        width: 320,
        height: 200,
        locked: false,
    };
    let resolved = uniwidged_host::placement::resolve(&stored, &outputs);
    assert!(!resolved.displaced);
    assert_eq!(resolved.output.as_deref(), Some("RIGHT-1"));
    assert_eq!(resolved.rect.x, 640 + 32);
    assert_eq!(resolved.rect.y, 24);

    // And a real window really opens there. Read back from the server, because
    // what a client asked for and what a window manager did are different
    // facts — and on this path there is no window manager, which is itself
    // worth pinning.
    let window = X11Window::open_on(
        Some(&server.display),
        resolved.rect,
        "right-hand",
        WindowKind::DesktopWidget,
    )
    .expect("a desktop-layer window");

    // Read from the server on a second connection, not from the window's own
    // record: what a client asked for and what is actually on screen are
    // different facts, and only one of them is what a user sees.
    let pointer = Pointer::open(&server.display);
    let reply = pointer
        .conn
        .get_geometry(window.window_id())
        .expect("geometry request")
        .reply()
        .expect("geometry");
    assert_eq!((reply.x, reply.y), (672, 24));
    assert_eq!((reply.width, reply.height), (320, 200));
}

#[test]
fn unplugging_a_monitor_moves_a_widget_and_keeps_its_name() {
    // The half that makes docking work, and the half that is easy to get
    // backwards. A widget whose monitor is gone has to appear somewhere
    // reachable — but its stored name must **not** be rewritten, or plugging
    // the monitor back in would leave it on the laptop screen for good.
    let Some(server) = server() else { return };
    if !two_monitors(&server.display) {
        return;
    }
    let stored = uniwidged_host::config::Placement {
        output: Some("RIGHT-1".into()),
        x: 32,
        y: 24,
        width: 320,
        height: 200,
        locked: false,
    };

    let docked = uniwidged_shell::platform::x11::outputs(Some(&server.display)).expect("outputs");
    let there = uniwidged_host::placement::resolve(&stored, &docked);
    assert_eq!(there.rect.x, 672);

    assert!(delete_monitor(&server.display, "RIGHT-1"), "unplug it");
    let undocked = uniwidged_shell::platform::x11::outputs(Some(&server.display)).expect("outputs");
    assert!(
        !undocked.iter().any(|output| output.name == "RIGHT-1"),
        "the monitor should be gone: {undocked:?}"
    );

    let displaced = uniwidged_host::placement::resolve(&stored, &undocked);
    assert!(displaced.displaced, "it should say the monitor is missing");
    // Somewhere on a screen that exists, and *within* it — a widget pushed off
    // the edge by an undock is a widget its user cannot get back.
    let landed_on = displaced
        .output
        .as_deref()
        .expect("it should say where it went");
    let fallback = undocked
        .iter()
        .find(|output| output.name == landed_on)
        .expect("and that output should exist");
    assert!(displaced.rect.x >= fallback.rect.x);
    assert!(displaced.rect.y >= fallback.rect.y);
    assert!(
        displaced.rect.x + displaced.rect.width as i32
            <= fallback.rect.x + fallback.rect.width as i32
    );
    assert!(
        displaced.rect.y + displaced.rect.height as i32
            <= fallback.rect.y + fallback.rect.height as i32
    );
    // And the configuration is untouched: the name is still the one the user
    // put it on.
    assert_eq!(stored.output.as_deref(), Some("RIGHT-1"));

    // Docking again puts it back, which is the whole point of not rewriting.
    assert!(
        set_monitor(&server.display, "RIGHT-1", "640/170x720/240+640+0"),
        "plug it back in"
    );
    let redocked = uniwidged_shell::platform::x11::outputs(Some(&server.display)).expect("outputs");
    let back = uniwidged_host::placement::resolve(&stored, &redocked);
    assert!(!back.displaced);
    assert_eq!(back.rect, there.rect, "it should be exactly where it was");
}

#[test]
fn dragging_a_widget_onto_the_other_monitor_records_the_other_monitor() {
    // The inverse of `resolve`, against real monitors: a widget moved across
    // the seam has to be recorded against the output it landed on and at
    // coordinates relative to *that* output. Getting this wrong is invisible
    // until the machine is undocked, and then everything is in the wrong place
    // at once.
    let Some(server) = server() else { return };
    if !two_monitors(&server.display) {
        return;
    }
    let outputs = uniwidged_shell::platform::x11::outputs(Some(&server.display)).expect("outputs");

    // Dropped well inside the right-hand monitor.
    let dropped = PhysicalRect::new(700, 100, 320, 200);
    let (output, relative) = uniwidged_host::placement::locate(dropped, &outputs);
    assert_eq!(output.as_deref(), Some("RIGHT-1"));
    assert_eq!((relative.x, relative.y), (60, 100));

    // And recording then resolving is the identity, which is the property a
    // sign error in either direction survives everything else.
    let stored = uniwidged_host::config::Placement {
        output,
        x: relative.x,
        y: relative.y,
        width: relative.width,
        height: relative.height,
        locked: false,
    };
    assert_eq!(
        uniwidged_host::placement::resolve(&stored, &outputs).rect,
        dropped
    );
}
