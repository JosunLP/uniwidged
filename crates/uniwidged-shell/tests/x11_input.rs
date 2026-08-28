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
        }
    }

    /// Pumps events for a while, doing what the desktop's own loop does.
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
    desktop.pump(Duration::from_millis(120));
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
