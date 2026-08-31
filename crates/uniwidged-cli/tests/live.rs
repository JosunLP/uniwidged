//! A desktop that is running while somebody changes it.
//!
//! Every other test in this crate drives the CLI against a file. This one
//! starts the real binary on a real X server with a real GPU device, puts a
//! real WASM widget on the screen, and then — with the desktop still up —
//! installs a second one, removes it again, and switches the first off.
//!
//! It exists because the interesting half of that feature is the half no unit
//! test can reach. `uniwidged_host::reload::apply` reports which widgets
//! appeared and disappeared and deliberately acts on neither: stopping one
//! means destroying a window, and the host does not own windows. What actually
//! creates and destroys them is `uniwidged_cli::run`, in an order that matters
//! — the renderer's surface has to be detached before the window under it goes,
//! or wgpu is presenting into something the X server has already forgotten.
//! Getting that wrong does not fail a type check. It fails on a screen.
//!
//! It is also where `uniwidged status` and `uniwidged log` are measured,
//! because both read what a running desktop wrote, and the only way to have one
//! of those is to run one.
//!
//! Without `Xvfb` or without a Vulkan device it skips, and `UNIWIDGED_REQUIRE_X11`
//! or `UNIWIDGED_REQUIRE_GPU` turn either skip into a failure — the same
//! bargain the golden images and the X11 pointer tests make.

#![cfg(all(unix, not(target_os = "macos")))]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long a line is waited for before the test gives up on it.
///
/// Generous. The desktop is compiling a WASM component and creating a
/// swapchain on a software rasteriser, and a loaded build machine is slow at
/// both. A test that measured *how fast* a widget appears would be measuring
/// the machine; what is being measured is whether it appears at all.
const PATIENCE: Duration = Duration::from_secs(60);

// --- The X server ---------------------------------------------------------

struct Server {
    display: String,
    number: u32,
    child: Child,
    /// A connection held for the server's whole life. An X server resets when
    /// its last client disconnects — that is what `-noreset` exists to turn
    /// off — and a desktop whose connection lands mid-reset is refused with
    /// an error that looks like no server at all. `x11_input.rs` holds one
    /// for the same reason and gave it the same name.
    _keepalive: x11rb::xcb_ffi::XCBConnection,
}

impl Server {
    fn start() -> Option<Self> {
        use std::sync::atomic::{AtomicU32, Ordering};
        // Above the range `uniwidged-shell`'s own tests use, so the two suites
        // can run at once.
        static NEXT: AtomicU32 = AtomicU32::new(140);
        for _ in 0..30 {
            let number = NEXT.fetch_add(1, Ordering::Relaxed);
            let display = format!(":{number}");
            if Path::new(&format!("/tmp/.X11-unix/X{number}")).exists() {
                continue;
            }
            let Ok(mut child) = Command::new("Xvfb")
                .args([
                    &display,
                    "-screen",
                    "0",
                    "1280x720x24",
                    "+extension",
                    "RENDER",
                    "+extension",
                    "Composite",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                return None;
            };
            // The socket appears before the server is ready to accept, so the
            // wait is on a *completed X handshake* rather than on the file —
            // and the connection is then kept, not closed. Both halves
            // matter, and each was learned from a red CI run: the first
            // version waited on the file and slept, which a loaded runner
            // outran; the second probed with a bare connect and dropped it,
            // which made the probe the server's last client — and an X
            // server resets when its last client leaves, so the desktop's
            // own connection landed mid-reset and was refused.
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if let Ok((keepalive, _)) = x11rb::xcb_ffi::XCBConnection::connect(Some(
                    &std::ffi::CString::new(display.clone()).expect("a display name"),
                )) {
                    return Some(Server {
                        display,
                        number,
                        child,
                        _keepalive: keepalive,
                    });
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        None
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(format!("/tmp/.X11-unix/X{}", self.number));
        let _ = std::fs::remove_file(format!("/tmp/.X{}-lock", self.number));
    }
}

/// Starts a server, or says why the test is not running.
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

/// Whether this session can put a swapchain on an X11 window — asked by
/// doing it, against this test's own server, through the same calls the
/// desktop makes: a desktop-layer window, `Renderer::new`, and a swapchain
/// surface attached to it.
///
/// `Renderer::new().is_ok()` was the first version, and it answered the wrong
/// question: a runner can hold an adapter that renders headlessly and cannot
/// present, and there the tests ran and every desktop failed with "the
/// surface supports no format". Failing here says why; failing there said
/// nothing about whose fault it was.
fn presentable(server: &Server) -> bool {
    use uniwidged_core::geom::PhysicalRect;
    use uniwidged_shell::platform::x11::{WindowKind, X11Window};
    let probe = (|| {
        let window = X11Window::open_on(
            Some(&server.display),
            PhysicalRect::new(0, 0, 64, 64),
            "probe",
            WindowKind::DesktopWidget,
        )
        .map_err(|e| e.to_string())?;
        let mut renderer = uniwidged_render::Renderer::new().map_err(|e| e.to_string())?;
        let (display, handle) = window.raw_handles();
        // SAFETY: the window outlives the surface — the surface is removed
        // two lines down, and the renderer is dropped before the window at
        // the end of this closure.
        let surface = unsafe { renderer.add_swapchain_surface(display, handle, 64, 64) }
            .map_err(|e| e.to_string())?;
        renderer.remove_surface(surface);
        Ok::<(), String>(())
    })();
    match probe {
        Ok(()) => true,
        Err(why) => {
            assert!(
                std::env::var("UNIWIDGED_REQUIRE_GPU").is_err(),
                "UNIWIDGED_REQUIRE_GPU is set and this session cannot present: {why}"
            );
            eprintln!(
                "skipping: this session cannot present ({why}). \
                 Set UNIWIDGED_REQUIRE_GPU=1 to make this a failure."
            );
            false
        }
    }
}

// --- The machine the desktop runs on --------------------------------------

/// A scratch configuration, and everything that hangs off it.
///
/// `UNIWIDGED_CONFIG` decides where the widget store, the data root, the
/// keychain *and the pid file* go — which is the property this test needs
/// twice over. The desktop and the `uniwidged install` that interrupts it are
/// two processes, and they find each other through that pid file.
struct Machine(PathBuf);

impl Machine {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "uniwidged-live-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Machine(path)
    }

    fn config(&self) -> PathBuf {
        self.0.join("config.toml")
    }

    /// Runs a command until what it prints satisfies `done`, or `PATIENCE`
    /// runs out — for the answers a desktop writes a moment after it says
    /// something, such as its state file.
    fn run_until(&self, args: &[&str], done: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let said = self.run(args);
            if done(&said) || Instant::now() > deadline {
                return said;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Runs one CLI command to completion and returns what it printed.
    fn run(&self, args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_uniwidged"))
            .args(args)
            .env("UNIWIDGED_CONFIG", self.config())
            .output()
            .expect("the CLI should be runnable");
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success(),
            "`uniwidged {}` failed: {said}",
            args.join(" ")
        );
        said
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Builds one of the repository's widgets into the machine's scratch directory.
///
/// Built rather than looked for, as everywhere else in this repository: a test
/// that skips when an artefact is missing eventually always skips.
fn build_widget(into: &Path, name: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../widgets")
        .join(name);
    let built = Command::new(env!("CARGO"))
        .current_dir(&source)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .status()
        .expect("cargo should be runnable");
    assert!(built.success(), "the {name} widget should build");
    let component = into.join(format!("{name}.wasm"));
    std::fs::copy(
        source
            .join("target/wasm32-wasip2/release")
            .join(format!("{}_widget.wasm", name.replace('-', "_"))),
        &component,
    )
    .expect("copy the component");
    std::fs::copy(source.join("widget.toml"), component.with_extension("toml"))
        .expect("copy the manifest");
    component
}

/// The desktop, running, with everything it printed readable as it prints it.
struct Running {
    child: Child,
    log: PathBuf,
}

impl Running {
    fn start(machine: &Machine, display: &str) -> Self {
        let log = machine.0.join("desktop.log");
        let out = std::fs::File::create(&log).expect("a log file");
        let err = out.try_clone().expect("the same file twice");
        let child = Command::new(env!("CARGO_BIN_EXE_uniwidged"))
            .arg("run")
            .env("UNIWIDGED_CONFIG", machine.config())
            .env("DISPLAY", display)
            // Belt and braces: `DISPLAY` decides, but a runner with a Wayland
            // session in its environment would otherwise send the desktop to
            // the compositor this test did not start.
            .env_remove("WAYLAND_DISPLAY")
            // So that a test which loses its way ends rather than running until
            // the harness times out. Comfortably longer than `PATIENCE`.
            .env("UNIWIDGED_RUN_SECS", "180")
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .spawn()
            .expect("the CLI should be runnable");
        Running { child, log }
    }

    fn said(&self) -> String {
        let mut text = String::new();
        if let Ok(mut file) = std::fs::File::open(&self.log) {
            let _ = file.read_to_string(&mut text);
        }
        text
    }

    /// Waits for a line the desktop prints, and panics with the whole log if it
    /// never comes.
    ///
    /// The log is the assertion message on purpose: a desktop that failed to
    /// start has already said why, and a bare "timed out" would throw that
    /// away and send whoever reads it looking.
    fn wait_for(&self, needle: &str) {
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            if self.said().contains(needle) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!(
            "the desktop never said `{needle}`. What it did say:\n{}",
            self.said()
        );
    }

    /// Asserts something has *not* happened, having given it time to.
    fn never_said(&self, needle: &str) {
        assert!(
            !self.said().contains(needle),
            "the desktop said `{needle}`:\n{}",
            self.said()
        );
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// --- The tests ------------------------------------------------------------

#[test]
fn a_widget_installed_while_the_desktop_runs_appears_without_a_restart() {
    let Some(server) = server() else { return };
    if !presentable(&server) {
        return;
    }
    let machine = Machine::new("install");
    let clock = build_widget(&machine.0, "clock");
    let counter = build_widget(&machine.0, "counter");

    machine.run(&["install", clock.to_str().unwrap(), "--allow-unsigned"]);
    let desktop = Running::start(&machine, &server.display);
    desktop.wait_for("1 widget(s) running");

    // The whole point: a second widget, installed by a *different process*,
    // against a desktop that is already on the screen. `uniwidged install`
    // writes the file and signals the pid it finds beside it.
    let said = machine.run(&["install", counter.to_str().unwrap(), "--allow-unsigned"]);
    assert!(
        said.contains("has been told"),
        "install should have found the running desktop: {said}"
    );
    desktop.wait_for("`counter` started");
    desktop.never_said("it starts when the desktop does");

    // And then away again, which is the half that has to undo a swapchain and
    // a window in the right order.
    machine.run(&["remove", "counter"]);
    desktop.wait_for("`counter` stopped");

    // The desktop is still up, still has the widget it started with, and did
    // not fall over doing any of it.
    machine.run(&["list"]);
    desktop.never_said("panicked");
}

#[test]
fn disabling_a_widget_takes_it_off_the_screen_and_enabling_puts_it_back() {
    // The difference between `disable` and `remove` is that a disabled widget
    // keeps its placement, its grants and its data — so this is the one that
    // proves a widget can come *back*, over the same configuration entry and
    // into a new window.
    let Some(server) = server() else { return };
    if !presentable(&server) {
        return;
    }
    let machine = Machine::new("disable");
    let clock = build_widget(&machine.0, "clock");
    let counter = build_widget(&machine.0, "counter");

    machine.run(&["install", clock.to_str().unwrap(), "--allow-unsigned"]);
    machine.run(&["install", counter.to_str().unwrap(), "--allow-unsigned"]);
    let desktop = Running::start(&machine, &server.display);
    desktop.wait_for("2 widget(s) running");

    machine.run(&["disable", "counter"]);
    desktop.wait_for("`counter` stopped");

    machine.run(&["enable", "counter"]);
    desktop.wait_for("`counter` started");
    desktop.never_said("panicked");
}

#[test]
fn a_desktop_whose_last_widget_is_removed_keeps_running_and_can_be_refilled() {
    // The edge the window list has: with nothing on the screen there is no
    // window to wait on, and the loop still has to block rather than spin.
    // Before `wait_any` took a slice, this was an index into an empty vector.
    let Some(server) = server() else { return };
    if !presentable(&server) {
        return;
    }
    let machine = Machine::new("empty");
    let clock = build_widget(&machine.0, "clock");

    machine.run(&["install", clock.to_str().unwrap(), "--allow-unsigned"]);
    let desktop = Running::start(&machine, &server.display);
    desktop.wait_for("1 widget(s) running");

    machine.run(&["remove", "clock"]);
    desktop.wait_for("`clock` stopped");

    // Still alive with an empty screen — and it has to have *blocked*, not
    // spun, which is what makes the next line arrive at all rather than the
    // process burning a core.
    machine.run(&["install", clock.to_str().unwrap(), "--allow-unsigned"]);
    desktop.wait_for("`clock` started");
    desktop.never_said("panicked");
}

#[test]
fn status_and_log_report_what_a_running_desktop_is_doing() {
    // `list` reads a file that says what somebody asked for; `status` reads
    // what the desktop managed to do. The second is only worth anything if it
    // follows the desktop — a widget that leaves the screen has to leave the
    // answer — and if it is refused once the desktop is gone.
    let Some(server) = server() else { return };
    if !presentable(&server) {
        return;
    }
    let machine = Machine::new("status");
    let clock = build_widget(&machine.0, "clock");
    machine.run(&["install", clock.to_str().unwrap(), "--allow-unsigned"]);

    // Nothing running: a sentence, not an error, and `list` says nothing
    // about the screen because there is no screen to say anything about.
    assert!(machine.run(&["status"]).contains("no desktop is running"));
    assert!(!machine.run(&["list"]).contains("on screen"));

    let desktop = Running::start(&machine, &server.display);
    desktop.wait_for("1 widget(s) running");

    // The state file is written just after that line, so the answer is polled
    // for rather than asserted on the first read.
    let said = machine.run_until(&["status"], |s| s.contains("clock"));
    assert!(said.contains("clock 1.0.0  running"), "{said}");
    assert!(said.contains("shell:"), "{said}");
    assert!(said.contains(" at "), "{said}");
    let listed = machine.run(&["list"]);
    assert!(listed.contains("clock (on screen)"), "{listed}");

    // Everything the desktop printed is in the log too, with a timestamp in
    // front of it — the terminal being the half that usually does not exist.
    let log = machine.run(&["log"]);
    assert!(log.contains("widget(s) running"), "{log}");
    assert!(log.contains("shell:"), "{log}");
    let stamped = log
        .lines()
        .find(|line| line.contains("widget(s) running"))
        .expect("the line");
    assert!(
        stamped.len() > 19 && stamped.as_bytes()[4] == b'-' && stamped.as_bytes()[13] == b':',
        "no timestamp on {stamped:?}"
    );

    // A widget that leaves the screen leaves the answer.
    machine.run(&["disable", "clock"]);
    desktop.wait_for("`clock` stopped");
    let said = machine.run_until(&["status"], |s| s.contains("nothing on the screen"));
    assert!(!said.contains("clock 1.0.0"), "{said}");
    let listed = machine.run(&["list"]);
    assert!(listed.contains("clock (disabled)"), "{listed}");
    assert!(
        !listed.contains("not running"),
        "a disabled widget is not also `not running`: {listed}"
    );

    // And a desktop killed outright — `Drop` sends SIGKILL — leaves its pid
    // file and its state file behind, and neither is believed.
    drop(desktop);
    let said = machine.run_until(&["status"], |s| s.contains("no desktop is running"));
    assert!(!said.contains("shell:"), "{said}");
}
