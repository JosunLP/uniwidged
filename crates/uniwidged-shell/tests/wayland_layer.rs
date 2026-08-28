//! A layer surface, measured against a real compositor.
//!
//! `spikes/m0_layer_shell` answered the question this platform rests on — can a
//! `zwlr_layer_shell_v1` surface back a GPU swapchain with per-pixel alpha —
//! with screenshots and arithmetic. What it could not do is keep answering it,
//! because a spike is run by hand.
//!
//! This is the standing half. It starts its own headless `sway`, creates the
//! surface `uniwidged run` creates, and checks the facts only a compositor can
//! confirm: that the layer shell accepted the role, that the configure
//! handshake completed with a size *the compositor* chose, that a placement
//! survives a round trip through it, and that the monitor comes back with a
//! name a placement can be stored against.
//!
//! Without `sway` it skips; `UNIWIDGED_REQUIRE_WAYLAND=1` turns that into a
//! failure, and CI sets it — the same bargain the golden images and the X11
//! pointer tests make.

#![cfg(all(unix, not(target_os = "macos")))]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use uniwidged_core::geom::PhysicalRect;
use uniwidged_shell::platform::wayland::{Prefer, Shape, WaylandWindow};

const START: PhysicalRect = PhysicalRect::new(48, 48, 320, 200);

/// A headless sway, stopped when the test ends.
struct Compositor {
    socket: PathBuf,
    runtime: PathBuf,
    child: Child,
}

impl Compositor {
    /// `None` when there is no `sway` to start.
    fn start(what: &str) -> Option<Self> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        // Short by construction, and that is load-bearing. sway puts its IPC
        // socket at `$XDG_RUNTIME_DIR/sway-ipc.<uid>.<pid>.sock`, and a Unix
        // socket path cannot exceed 108 bytes — past that sway logs one line
        // and segfaults, which from a client looks exactly like "no
        // compositor". `TMPDIR` on a build machine is routinely long enough to
        // trigger it, so this uses it only when it leaves room.
        let name = format!(
            "uw-wl-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let base = std::env::temp_dir();
        // `sway-ipc.` + uid + pid + `.sock` and separators. Forty is generous
        // and still leaves the check meaningful.
        let runtime = if base.as_os_str().len() + name.len() + 40 < 108 {
            base.join(&name)
        } else {
            PathBuf::from("/tmp").join(&name)
        };
        let _ = std::fs::remove_dir_all(&runtime);
        std::fs::create_dir_all(&runtime).ok()?;
        // A runtime directory must be private, and sway refuses one that is
        // not.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700));
        }

        let config = runtime.join("sway.conf");
        std::fs::write(
            &config,
            // A saturated background is what a widget has to be transparent
            // against, and it is what makes the spike's proof arithmetic. Here
            // it only has to exist.
            "output HEADLESS-1 mode 1280x720\n\
             output HEADLESS-1 background #1E6FD9 solid_color\n\
             default_border none\n\
             focus_follows_mouse no\n",
        )
        .ok()?;

        let child = Command::new("sway")
            .arg("-c")
            .arg(&config)
            // Its own runtime directory, so the socket is findable and no
            // session on this machine is disturbed. The environment goes to
            // the child only: setting `WAYLAND_DISPLAY` in *this* process
            // would be a data race, which is why `open_on` takes a path.
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            // Software rendering: this test is about the protocol, not the GPU,
            // and a runner without a device should still run it.
            .env("WLR_RENDERER", "pixman")
            .env("XDG_SESSION_TYPE", "wayland")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let mut compositor = Compositor {
            socket: PathBuf::new(),
            runtime,
            child,
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if let Some(socket) = compositor.find_socket() {
                compositor.socket = socket;
                return Some(compositor);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    fn find_socket(&self) -> Option<PathBuf> {
        std::fs::read_dir(&self.runtime).ok()?.find_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            (name.starts_with("wayland-") && !name.ends_with(".lock")).then_some(path)
        })
    }

    fn open(&self, rect: PhysicalRect, prefer: Prefer) -> WaylandWindow {
        WaylandWindow::open_on(Some(&self.socket), rect, "uniwidged test", prefer)
            .expect("a widget surface")
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.runtime);
    }
}

/// Starts a compositor, or says why the test is not running.
fn compositor(what: &str) -> Option<Compositor> {
    match Compositor::start(what) {
        Some(compositor) => Some(compositor),
        None => {
            assert!(
                std::env::var("UNIWIDGED_REQUIRE_WAYLAND").is_err(),
                "UNIWIDGED_REQUIRE_WAYLAND is set and no headless sway could be started"
            );
            eprintln!("skipping: no sway. Set UNIWIDGED_REQUIRE_WAYLAND=1 to make this a failure.");
            None
        }
    }
}

#[test]
fn a_widget_gets_a_layer_surface_and_the_compositor_configures_it() {
    // The claim the whole platform rests on, and the one a unit test cannot
    // make: a compositor accepted the `BOTTOM` layer role and completed the
    // handshake. `open` returns only if it did.
    let Some(compositor) = compositor("layer") else {
        return;
    };
    let window = compositor.open(START, Prefer::LayerShell);

    assert_eq!(window.shape(), Shape::Layer, "sway has a layer shell");
    let capabilities = window.capabilities();
    assert!(capabilities.below_windows, "that is what BOTTOM means");
    assert!(capabilities.self_placing);
    assert!(capabilities.per_pixel_alpha);
    assert_eq!(capabilities.degradation(), None);

    // The size came back from the compositor rather than from us. A layer
    // surface asked for one and was granted it; a compositor with less room
    // would have said so, and the caller has to take that answer.
    assert_eq!(window.geometry().width, START.width);
    assert_eq!(window.geometry().height, START.height);
    assert!(window.has_compositor());
}

#[test]
fn a_placement_survives_a_round_trip_through_the_compositor() {
    // There is no `ConfigureWindow` here: a position is an anchor and two
    // margins, and the only way to know the translation is right is to send it
    // and be reconfigured. Sizes are what a compositor answers with, so a size
    // change is the observable half.
    let Some(compositor) = compositor("placement") else {
        return;
    };
    let mut window = compositor.open(START, Prefer::LayerShell);

    let moved = PhysicalRect::new(200, 120, 420, 300);
    window.set_geometry(moved).expect("set geometry");
    assert_eq!(window.geometry(), moved);

    // Pump until the compositor *announces* the new size. What is asserted is
    // the announcement rather than the client's own record — the record is
    // what `set_geometry` wrote, and asserting it would prove only that the
    // assignment happened.
    //
    // The first version of this loop asked whether `geometry()` had reached
    // the requested size and whether `take_resize` was quiet. It failed once
    // under load, with the size it started with, and it was a real bug rather
    // than a slow machine: `take_resize` compared the compositor's last known
    // size against the *requested* one, so in the gap before the answering
    // configure arrived it read the client's own pending request as a resize
    // and reverted it. See `WaylandWindow::reported`.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut announced = None;
    while Instant::now() < deadline && announced != Some((moved.width, moved.height)) {
        window.wait(Some(Duration::from_millis(50))).expect("wait");
        let _ = window.drain_events().expect("events");
        if let Some(size) = window.take_resize() {
            announced = Some(size);
        }
    }
    assert_eq!(
        announced,
        Some((moved.width, moved.height)),
        "the compositor should have granted the new size"
    );
    assert_eq!(
        (window.geometry().width, window.geometry().height),
        (moved.width, moved.height),
        "and the window's own record should agree with it"
    );
}

#[test]
fn a_pending_resize_is_not_mistaken_for_the_compositor_answering() {
    // The bug the test above found, pinned directly rather than left to a
    // race. Between asking for a size and being told, `take_resize` must say
    // *nothing*: reporting a resize there means reverting the client's own
    // request to the size it had, and then never asking again.
    let Some(compositor) = compositor("pending") else {
        return;
    };
    let mut window = compositor.open(START, Prefer::LayerShell);
    // Drain whatever the handshake left, so the compositor has nothing
    // outstanding to say.
    let _ = window.drain_events().expect("events");
    assert_eq!(window.take_resize(), None, "nothing has changed yet");

    let moved = PhysicalRect::new(0, 0, 500, 400);
    window.set_geometry(moved).expect("set geometry");
    // Deliberately *without* pumping: the compositor has not answered, so
    // there is nothing for it to have announced.
    assert_eq!(
        window.take_resize(),
        None,
        "an unanswered request is not a resize"
    );
    assert_eq!(
        (window.geometry().width, window.geometry().height),
        (moved.width, moved.height),
        "and the request must survive being asked about"
    );
}

#[test]
fn the_monitor_comes_back_with_a_name_a_placement_can_be_stored_against() {
    // `uniwidged_host::placement` stores a position relative to a named
    // output, because a name is the only identifier that survives docking.
    // This is the Wayland end of supplying one.
    let Some(compositor) = compositor("outputs") else {
        return;
    };
    let window = compositor.open(START, Prefer::LayerShell);

    let outputs = window.outputs();
    assert_eq!(outputs.len(), 1, "one headless output: {outputs:?}");
    assert_eq!(outputs[0].name, "HEADLESS-1");
    assert_eq!(outputs[0].rect, PhysicalRect::new(0, 0, 1280, 720));
    assert_eq!(window.desktop_bounds(), PhysicalRect::new(0, 0, 1280, 720));
}

#[test]
fn the_toplevel_fallback_still_gets_a_surface_and_says_what_it_lost() {
    // What a GNOME session gets. Mutter has no layer shell and has said it
    // will not, so this branch is the one a large share of users take — and it
    // would never run on a machine with sway on it unless something asked for
    // it deliberately.
    let Some(compositor) = compositor("toplevel") else {
        return;
    };
    let window = compositor.open(START, Prefer::Toplevel);

    assert_eq!(window.shape(), Shape::Toplevel);
    let capabilities = window.capabilities();
    assert!(!capabilities.below_windows, "it is among the windows now");
    assert!(!capabilities.self_placing, "and cannot place itself");
    assert!(
        capabilities.per_pixel_alpha,
        "what it must never give up: a widget on an opaque rectangle is not a widget"
    );
    assert!(capabilities.degradation().is_some());
}

#[test]
fn a_widget_that_cannot_reach_a_compositor_says_so_rather_than_hanging() {
    // The first thing that happens on a machine with no Wayland session, and
    // the message is what a user sees.
    let Err(error) = WaylandWindow::open_on(
        Some(Path::new("/nonexistent/wayland-99")),
        START,
        "nowhere",
        Prefer::LayerShell,
    ) else {
        panic!("there is nothing at that path to connect to");
    };
    let said = error.to_string();
    assert!(said.contains("no Wayland display"), "{said}");
    assert!(said.contains("wayland-99"), "and which socket: {said}");
}

#[test]
fn an_idle_widget_waits_without_spinning() {
    // Wayland's half of `ARCHITECTURE.md` §5. Nothing is happening, so the
    // loop must actually block rather than turn as fast as the machine can.
    //
    // What is asserted is the *loop*, not one `wait`, and the difference is a
    // flake this file used to have. A single wait can return early and be
    // perfectly correct: the compositor sends the client something — a frame
    // callback, an output change, a serial — and `poll` reports the socket
    // readable, which is the mechanism working. What would be a busy loop is
    // that happening again and again with nothing behind it.
    //
    // So the test drains, waits, and repeats: an early return is allowed as
    // long as one of a small number of waits genuinely blocks. It failed under
    // a full-workspace run and passed alone, which is exactly what an
    // assertion about a single sample looks like when the sample is noisy.
    let Some(compositor) = compositor("idle") else {
        return;
    };
    let mut window = compositor.open(START, Prefer::LayerShell);

    const ATTEMPTS: usize = 5;
    let mut early = Vec::new();
    for _ in 0..ATTEMPTS {
        // Whatever woke the last attempt is consumed here, so an event
        // delivered once cannot end every wait.
        let _ = window.drain_events().expect("events");
        let began = Instant::now();
        window.wait(Some(Duration::from_millis(150))).expect("wait");
        let waited = began.elapsed();
        if waited >= Duration::from_millis(100) {
            return;
        }
        early.push(waited);
    }
    panic!(
        "{ATTEMPTS} waits in a row returned early ({early:?}) with nothing          happening, which is a busy loop"
    );
}
