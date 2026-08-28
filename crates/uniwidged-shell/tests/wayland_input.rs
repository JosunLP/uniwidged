//! Pointer input on Wayland, measured rather than asserted.
//!
//! This is the gap M7 named and left open: `platform::wayland` binds a seat,
//! takes its `wl_pointer` and turns enter/motion/button into the same [`Input`]
//! values X11 produces — and until now nothing had ever seen one arrive.
//! Headless sway has no mouse, so there was nothing to move.
//!
//! There is a way, and it is the Wayland equivalent of what the X11 suite does
//! with `xtest`: `zwlr_virtual_pointer_manager_v1`, which wlroots implements
//! and sway therefore has. A client creates a pointer device, and every event
//! it sends goes through the compositor's ordinary input handling — focus,
//! surface-local coordinates, the implicit grab — before coming back out to
//! whichever surface is under it. So what is being measured is the whole path,
//! not a shortcut into it.
//!
//! **What this deliberately does not test** is the interaction model above the
//! plumbing — what a press *means*, whether it moves a widget or reaches a
//! button. That is `uniwidged_host::interaction`, it is one implementation for
//! both platforms, and `x11_input.rs` drives it end to end. Testing it twice
//! would be testing the same rules against two transports; what is worth
//! measuring here is only the transport.
//!
//! Without `sway` it skips; `UNIWIDGED_REQUIRE_WAYLAND=1` turns that into a
//! failure, and CI sets it.

#![cfg(all(unix, not(target_os = "macos")))]

use std::time::{Duration, Instant};

use uniwidged_core::geom::PhysicalRect;
use uniwidged_shell::Input;
use uniwidged_shell::platform::wayland::{Prefer, WaylandWindow};

use wayland_client::protocol::{wl_pointer, wl_registry};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

mod wayland_harness;
use wayland_harness::compositor;

/// The output the harness configures, which every absolute coordinate below is
/// a fraction of.
const OUTPUT: (u32, u32) = (1280, 720);

/// Where the widget sits on it.
const WIDGET: PhysicalRect = PhysicalRect::new(120, 80, 400, 260);

/// The evdev code for the left button, which is what Wayland passes through.
const BTN_LEFT: u32 = 0x110;

// --- A mouse that does not exist ------------------------------------------

/// A `zwlr_virtual_pointer_v1` on its own connection.
///
/// Its own, and not the widget's, for the reason that makes the whole test
/// meaningful: events have to leave this process, be routed by the compositor
/// and arrive at the widget through the compositor's own dispatch. Sending them
/// on the widget's connection would still work — the protocol does not care —
/// but it would put the client and the input source in one event queue, and an
/// ordering that held only because of that is one that would not hold on a real
/// desktop.
struct VirtualPointer {
    connection: Connection,
    queue: wayland_client::EventQueue<Manager>,
    pointer: ZwlrVirtualPointerV1,
    /// Wayland timestamps are milliseconds from an arbitrary epoch, and a
    /// compositor is entitled to ignore an event that goes backwards in time.
    clock: u32,
}

#[derive(Default)]
struct Manager {
    manager: Option<ZwlrVirtualPointerManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Manager {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == "zwlr_virtual_pointer_manager_v1"
        {
            state.manager = Some(registry.bind(name, version.min(2), qh, ()));
        }
    }
}

delegate_noop!(Manager: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(Manager: ignore ZwlrVirtualPointerV1);

impl VirtualPointer {
    /// `None` when this compositor has no virtual-pointer manager.
    fn attach(socket: &std::path::Path) -> Option<Self> {
        let stream = std::os::unix::net::UnixStream::connect(socket).ok()?;
        let connection = Connection::from_socket(stream).ok()?;
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        let _registry = connection.display().get_registry(&qh, ());
        let mut manager = Manager::default();
        queue.roundtrip(&mut manager).ok()?;
        let manager = manager.manager?;
        // No seat: the manager places it on the compositor's only one, which is
        // what a headless session has.
        let pointer = manager.create_virtual_pointer(None, &qh, ());
        let mut this = VirtualPointer {
            connection,
            queue,
            pointer,
            clock: 1,
        };
        // Flushed here, and the ordering it buys is the whole reason this is
        // not left to the first `move_to`.
        //
        // A headless seat has **no pointer capability at all** until a virtual
        // one is created — `wl_seat.capabilities` is literally zero — and a
        // client binds its `wl_pointer` from that event. A widget that opened
        // its surface while the capability was still absent would never take a
        // pointer, and every event below would go to a compositor with nowhere
        // to deliver it. Which is exactly what the first version of this file
        // did, and it failed with an empty event list and no other clue.
        this.flush();
        Some(this)
    }

    fn tick(&mut self) -> u32 {
        self.clock += 10;
        self.clock
    }

    /// Puts the pointer at a point on the output, in output pixels.
    ///
    /// Absolute rather than relative, because a headless compositor has no
    /// idea where a cursor "was" and a relative move from an unknown origin
    /// lands somewhere unknown.
    fn move_to(&mut self, x: u32, y: u32) {
        let time = self.tick();
        self.pointer.motion_absolute(time, x, y, OUTPUT.0, OUTPUT.1);
        self.pointer.frame();
        self.flush();
    }

    fn press(&mut self) {
        let time = self.tick();
        self.pointer
            .button(time, BTN_LEFT, wl_pointer::ButtonState::Pressed);
        self.pointer.frame();
        self.flush();
    }

    fn release(&mut self) {
        let time = self.tick();
        self.pointer
            .button(time, BTN_LEFT, wl_pointer::ButtonState::Released);
        self.pointer.frame();
        self.flush();
    }

    /// Sends what has been queued and waits for the compositor to have read it.
    ///
    /// A `roundtrip` rather than a `flush`: it returns only once the compositor
    /// has answered a `wl_display.sync` issued *after* these events, which is
    /// the protocol's guarantee that it has processed them. Without it the test
    /// would race the compositor and fail on a loaded machine — which is the
    /// most expensive kind of test to own.
    fn flush(&mut self) {
        let mut manager = Manager::default();
        let _ = self.queue.roundtrip(&mut manager);
        let _ = self.connection.flush();
    }
}

// --- Reading what arrived --------------------------------------------------

/// Pumps the widget's connection until its events satisfy `wanted`, or gives up.
///
/// Accumulating rather than draining once: one `wait_with` may return with only
/// the enter, and a test that looked at the first batch and gave up would pass
/// or fail depending on how the compositor happened to split its writes.
fn collect(
    window: &mut WaylandWindow,
    within: Duration,
    wanted: impl Fn(&[Input]) -> bool,
) -> Vec<Input> {
    let deadline = Instant::now() + within;
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        window
            .wait_with(Some(Duration::from_millis(50)), None)
            .expect("wait");
        seen.extend(window.drain_events().expect("drain"));
        if wanted(&seen) {
            return seen;
        }
    }
    seen
}

fn pressed_at(events: &[Input]) -> Option<((i32, i32), (i32, i32))> {
    events.iter().find_map(|input| match input {
        Input::Pressed { local, root, .. } => Some((*local, *root)),
        _ => None,
    })
}

/// Starts a compositor and attaches a virtual pointer, or says why not.
fn fixtures(what: &str) -> Option<(wayland_harness::Compositor, VirtualPointer)> {
    let compositor = compositor(what)?;
    match VirtualPointer::attach(&compositor.socket) {
        Some(pointer) => Some((compositor, pointer)),
        None => {
            // A compositor without the protocol is a real possibility — it is
            // a wlroots extension, not a standard — so this is a skip rather
            // than a failure, with the same escape hatch as the rest.
            assert!(
                std::env::var("UNIWIDGED_REQUIRE_WAYLAND").is_err(),
                "UNIWIDGED_REQUIRE_WAYLAND is set and this compositor has no \
                 zwlr_virtual_pointer_manager_v1"
            );
            eprintln!("skipping: no virtual pointer manager.");
            None
        }
    }
}

// --- The tests -------------------------------------------------------------

#[test]
fn a_press_on_a_widget_arrives_with_coordinates_the_host_can_use() {
    let Some((compositor, mut mouse)) = fixtures("press") else {
        return;
    };
    let mut window = compositor.open(WIDGET, Prefer::LayerShell);
    // The surface has to be on screen before anything can be over it: a layer
    // surface is not mapped until a buffer is attached, and until then the
    // compositor has nowhere to send a pointer.
    window.present_blank().expect("a first frame");

    // The middle of the widget, in output coordinates.
    let target = (
        WIDGET.x as u32 + WIDGET.width / 2,
        WIDGET.y as u32 + WIDGET.height / 2,
    );
    mouse.move_to(target.0, target.1);
    mouse.press();

    let events = collect(&mut window, Duration::from_secs(10), |seen| {
        pressed_at(seen).is_some()
    });
    let (local, root) = pressed_at(&events)
        .unwrap_or_else(|| panic!("no press reached the widget; what did: {events:?}"));

    // **Surface-local**, which is the whole reason this had to be measured.
    // Wayland never tells a client where the pointer is on the desktop — it
    // reports a position on the surface — and the host hit-tests in exactly
    // these coordinates. An off-by-the-window-origin here would put every
    // button in the wrong place, and would look like a layout bug.
    assert_eq!(
        local,
        ((WIDGET.width / 2) as i32, (WIDGET.height / 2) as i32),
        "the press should be in the middle of the surface, not of the screen"
    );

    // And the desktop coordinates, which `platform::wayland` derives by adding
    // the placement's origin — because the protocol will not give them and the
    // interaction model needs them to drag anything.
    assert_eq!(
        root,
        (target.0 as i32, target.1 as i32),
        "the desktop position should be the surface position plus the origin"
    );
}

#[test]
fn a_drag_across_the_widget_arrives_as_motion_and_a_release() {
    // The three events a move is made of, in order, from a pointer that never
    // leaves the surface. What is being checked is that motion is delivered
    // *while a button is down* and that the release closes the sequence — the
    // shape `uniwidged_host::interaction` consumes.
    let Some((compositor, mut mouse)) = fixtures("drag") else {
        return;
    };
    let mut window = compositor.open(WIDGET, Prefer::LayerShell);
    window.present_blank().expect("a first frame");

    let from = (WIDGET.x as u32 + 40, WIDGET.y as u32 + 40);
    let to = (WIDGET.x as u32 + 200, WIDGET.y as u32 + 160);
    mouse.move_to(from.0, from.1);
    mouse.press();
    // Several steps, as a real drag is: one jump would not distinguish
    // "motion is delivered" from "the release carried a new position".
    for step in 1..=4 {
        let x = from.0 + (to.0 - from.0) * step / 4;
        let y = from.1 + (to.1 - from.1) * step / 4;
        mouse.move_to(x, y);
    }
    mouse.release();

    let events = collect(&mut window, Duration::from_secs(10), |seen| {
        seen.iter().any(|e| matches!(e, Input::Released { .. }))
    });

    let press = events
        .iter()
        .position(|e| matches!(e, Input::Pressed { .. }))
        .unwrap_or_else(|| panic!("no press: {events:?}"));
    let release = events
        .iter()
        .position(|e| matches!(e, Input::Released { .. }))
        .unwrap_or_else(|| panic!("no release: {events:?}"));
    assert!(press < release, "the release came first: {events:?}");

    let motions: Vec<_> = events[press..release]
        .iter()
        .filter_map(|e| match e {
            Input::Motion { root, .. } => Some(*root),
            _ => None,
        })
        .collect();
    assert!(
        !motions.is_empty(),
        "no motion between the press and the release: {events:?}"
    );
    // The last one is the only position that describes where the pointer
    // actually ended up, which is what the desktop's loop acts on after
    // draining a burst.
    assert_eq!(
        *motions.last().expect("a motion"),
        (to.0 as i32, to.1 as i32),
        "the last motion should be where the drag finished"
    );

    match events[release] {
        Input::Released { root, .. } => assert_eq!(root, (to.0 as i32, to.1 as i32)),
        ref other => panic!("{other:?}"),
    }
}

#[test]
fn a_press_somewhere_else_on_the_desktop_is_not_delivered_to_the_widget() {
    // The half a client cannot see going wrong, and the reason Wayland reports
    // surface-local coordinates in the first place: a widget is told about the
    // pointer only while the pointer is on it. A press elsewhere belongs to
    // whatever is there, and a widget that received one would be watching the
    // whole desktop.
    let Some((compositor, mut mouse)) = fixtures("elsewhere") else {
        return;
    };
    let mut window = compositor.open(WIDGET, Prefer::LayerShell);
    window.present_blank().expect("a first frame");

    // Far away, and on the same output.
    mouse.move_to(OUTPUT.0 - 40, OUTPUT.1 - 40);
    mouse.press();
    mouse.release();

    let events = collect(&mut window, Duration::from_secs(2), |_| false);
    assert!(
        pressed_at(&events).is_none(),
        "a press outside the widget reached it: {events:?}"
    );
}

#[test]
fn a_pointer_that_leaves_during_a_drag_keeps_being_delivered() {
    // Wayland's *implicit grab*: from the moment a button goes down until it
    // comes up, every pointer event goes to the surface it went down on,
    // wherever the pointer travels. It is why `WaylandWindow::grab_pointer` is
    // a no-op and X11's is not, and it is a claim in that method's
    // documentation that nothing had checked.
    let Some((compositor, mut mouse)) = fixtures("grab") else {
        return;
    };
    let mut window = compositor.open(WIDGET, Prefer::LayerShell);
    window.present_blank().expect("a first frame");

    mouse.move_to(WIDGET.x as u32 + 20, WIDGET.y as u32 + 20);
    mouse.press();
    // Off the surface entirely, which is what a drag faster than the
    // compositor looks like.
    let outside = (WIDGET.x as u32 + WIDGET.width + 300, WIDGET.y as u32 + 20);
    mouse.move_to(outside.0, outside.1);
    mouse.release();

    let events = collect(&mut window, Duration::from_secs(10), |seen| {
        seen.iter().any(|e| matches!(e, Input::Released { .. }))
    });
    let released = events
        .iter()
        .find_map(|e| match e {
            Input::Released { root, .. } => Some(*root),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the grab did not hold: {events:?}"));
    assert_eq!(
        released,
        (outside.0 as i32, outside.1 as i32),
        "the release should have arrived from outside the surface"
    );
}
