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
            // The socket appears before the server accepts, so the wait is on
            // the file *and* a moment after it.
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if Path::new(&format!("/tmp/.X11-unix/X{number}")).exists() {
                    std::thread::sleep(Duration::from_millis(250));
                    return Some(Server {
                        display,
                        number,
                        child,
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

/// Whether this machine has a device the desktop can render on.
///
/// Asked before the server is started, and asked of the renderer rather than
/// guessed from an environment variable: the desktop will fail at exactly this
/// point if there is nothing, and failing here says why.
fn have_a_device() -> bool {
    if uniwidged_render::Renderer::new().is_ok() {
        return true;
    }
    assert!(
        std::env::var("UNIWIDGED_REQUIRE_GPU").is_err(),
        "UNIWIDGED_REQUIRE_GPU is set and no adapter could be found"
    );
    eprintln!("skipping: no GPU adapter. Set UNIWIDGED_REQUIRE_GPU=1 to make this a failure.");
    false
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
    if !have_a_device() {
        return;
    }
    let Some(server) = server() else { return };
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
    if !have_a_device() {
        return;
    }
    let Some(server) = server() else { return };
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
    if !have_a_device() {
        return;
    }
    let Some(server) = server() else { return };
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
