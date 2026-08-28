//! A headless `sway`, started by the test that needs one.
//!
//! Shared by every Wayland suite in this crate rather than copied into each:
//! two copies of a compositor harness is two harnesses drifting apart, and the
//! socket-length rule below is exactly the kind of hard-won detail that would
//! be fixed in one of them and not the other.
//!
//! Not `#[cfg(test)]` and not in `src/` — an integration test cannot see a
//! crate's unit-test modules, so a helper shared between two of them is a
//! module under `tests/`.

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use uniwidged_core::geom::PhysicalRect;
use uniwidged_shell::platform::wayland::{Prefer, WaylandWindow};

/// A headless sway, stopped when the test ends.
pub struct Compositor {
    pub socket: PathBuf,
    pub runtime: PathBuf,
    child: Child,
}

impl Compositor {
    /// `None` when there is no `sway` to start.
    pub fn start(what: &str) -> Option<Self> {
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

    pub fn open(&self, rect: PhysicalRect, prefer: Prefer) -> WaylandWindow {
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
pub fn compositor(what: &str) -> Option<Compositor> {
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
