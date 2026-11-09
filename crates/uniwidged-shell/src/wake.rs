//! How a background thread interrupts a blocking event loop.
//!
//! An idle desktop blocks on its window system's socket with **no timer at
//! all** — that is what `ARCHITECTURE.md` §5 is about, and it is why eight
//! widgets doing nothing cost nothing. The cost of that is that nothing else
//! can make the loop turn.
//!
//! A widget's asynchronous work finishes on a worker thread, and the answer is
//! useless until the loop notices. So the loop waits on two descriptors: the
//! display socket, and the read end of this pipe. A worker writes one byte, the
//! poll returns, and the host drains its finished tasks.
//!
//! ## Why a pipe
//!
//! Because it is the only thing a `poll` set can hold that another thread can
//! touch without a lock. A condition variable cannot be polled beside a socket;
//! a timeout turns an idle desktop into a polling one, which is the single
//! thing this project is most careful not to be. `eventfd` would be marginally
//! cheaper and exists only on Linux, and this file is the Unix one.
//!
//! Writes are non-blocking and a full pipe is *not* an error: a full pipe means
//! several wake-ups are already pending, and one is as good as ten — the host
//! drains everything when it wakes.
//!
//! ## The other two platforms
//!
//! **Windows** has no pipe a message loop can wait on, and does not need one:
//! `MsgWaitForMultipleObjectsEx` waits on the thread's message queue *and* on
//! kernel handles at once. So there the wake-up is a manual-reset event —
//! `SetEvent` is as lock-free and as thread-safe as a one-byte write, and
//! "full" cannot happen at all.
//!
//! **macOS** keeps the pipe, because `SIGHUP` still means *read your
//! configuration again* there and a signal handler may only `write`. But the
//! loop does not `poll`: it sits in `-[NSApplication nextEventMatchingMask:…]`,
//! which returns for an event and for nothing else. So a small thread blocks on
//! the read end, empties it, and posts one application-defined event — which
//! is the one thing AppKit documents as safe to do from another thread. The
//! loop's own [`WakePipe::drain`] then has nothing left to do.

use std::io;
use std::sync::Arc;

#[cfg(unix)]
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};

/// The read end, held by the loop; and a writer, cloned to workers.
#[cfg(unix)]
pub struct WakePipe {
    read: Arc<OwnedFd>,
    write: Arc<OwnedFd>,
}

#[cfg(unix)]
impl WakePipe {
    pub fn new() -> io::Result<Self> {
        let (read, write) = pipe()?;
        let pipe = Self {
            read: Arc::new(read),
            write: Arc::new(write),
        };
        #[cfg(target_os = "macos")]
        pipe.forward_to_appkit()?;
        Ok(pipe)
    }

    /// What the loop adds to its `poll` set.
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        // `OwnedFd` borrows as itself; named for what the caller wants.
        use std::os::fd::AsFd;
        self.read.as_fd()
    }

    /// The write end, for a signal handler — which can hold neither a closure
    /// nor a lock, only a descriptor to `write` one byte to. Valid for as long
    /// as this pipe is.
    pub fn write_fd(&self) -> RawFd {
        self.write.as_raw_fd()
    }

    /// A closure a worker thread can call to wake the loop.
    ///
    /// Deliberately the smallest possible thing: one non-blocking write, no
    /// lock, no allocation. It runs on a thread the loop knows nothing about,
    /// and anything it took could be something the loop is holding.
    pub fn notifier(&self) -> Arc<dyn Fn() + Send + Sync> {
        let write = Arc::clone(&self.write);
        Arc::new(move || {
            let byte = [1u8];
            // SAFETY: a valid descriptor this closure keeps alive, and one
            // byte from a local. A failure means the pipe is full, which means
            // a wake-up is already pending.
            unsafe {
                libc::write(write.as_raw_fd(), byte.as_ptr().cast(), 1);
            }
        })
    }

    /// Empties the pipe.
    ///
    /// Called after the loop wakes. Not doing it would leave the descriptor
    /// readable forever, and the loop would spin rather than block — which is
    /// the exact failure this whole mechanism exists to avoid, arrived at from
    /// the other direction.
    pub fn drain(&self) {
        drain_fd(self.read.as_raw_fd());
    }

    /// Turns every byte written into one AppKit event. See the module notes.
    #[cfg(target_os = "macos")]
    fn forward_to_appkit(&self) -> io::Result<()> {
        let read = Arc::clone(&self.read);
        std::thread::Builder::new()
            .name("uniwidged-wake".into())
            .spawn(move || {
                loop {
                    let mut fds = [libc::pollfd {
                        fd: read.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    }];
                    // SAFETY: one descriptor this thread keeps alive.
                    let ready = unsafe { libc::poll(fds.as_mut_ptr(), 1, -1) };
                    if ready < 0 {
                        if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                            continue;
                        }
                        return;
                    }
                    drain_fd(read.as_raw_fd());
                    crate::platform::appkit::post_wake();
                }
            })?;
        Ok(())
    }
}

/// A non-blocking, close-on-exec pipe.
///
/// `pipe2` where it exists, which is everywhere but macOS; there the two flags
/// are set straight after, which is only a race against another thread's
/// `fork` — and this process starts its commands with `posix_spawn` through
/// `std::process`, which closes what it is not given.
#[cfg(unix)]
fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as RawFd; 2];
    #[cfg(not(target_os = "macos"))]
    {
        // SAFETY: `pipe2` writes two descriptors into the array it is given.
        // `O_CLOEXEC` so a widget's command does not inherit them —
        // `Commands` clears the environment but a descriptor is not
        // environment — and `O_NONBLOCK` so a full pipe never stalls a worker.
        let made = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
        if made != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    #[cfg(target_os = "macos")]
    {
        // SAFETY: `pipe` writes two descriptors; `fcntl` is given each of them.
        unsafe {
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                return Err(io::Error::last_os_error());
            }
            for fd in fds {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                let flags = libc::fcntl(fd, libc::F_GETFL);
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
    }
    // SAFETY: both are freshly created and owned by nothing else.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

#[cfg(unix)]
fn drain_fd(fd: RawFd) {
    let mut sink = [0u8; 64];
    loop {
        // SAFETY: a valid descriptor and a local buffer of the length given.
        // Non-blocking, so this returns -1/EAGAIN when empty.
        let read = unsafe { libc::read(fd, sink.as_mut_ptr().cast(), sink.len()) };
        if read <= 0 {
            return;
        }
    }
}

/// A manual-reset event: set by any thread, waited on by the message loop.
#[cfg(windows)]
pub struct WakePipe {
    event: Arc<Event>,
}

#[cfg(windows)]
struct Event(windows_sys::Win32::Foundation::HANDLE);

// SAFETY: an event handle is a kernel object, and `SetEvent`/`ResetEvent` are
// documented as callable from any thread.
#[cfg(windows)]
unsafe impl Send for Event {}
#[cfg(windows)]
unsafe impl Sync for Event {}

#[cfg(windows)]
impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: a handle this struct owns and nobody else closes.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(windows)]
impl WakePipe {
    pub fn new() -> io::Result<Self> {
        use windows_sys::Win32::System::Threading::CreateEventW;
        // SAFETY: no attributes, manual reset, initially clear, unnamed.
        let handle = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            event: Arc::new(Event(handle)),
        })
    }

    /// What the loop adds to `MsgWaitForMultipleObjectsEx`.
    pub fn as_handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.event.0
    }

    /// As on Unix: one call, no lock, no allocation.
    pub fn notifier(&self) -> Arc<dyn Fn() + Send + Sync> {
        let event = Arc::clone(&self.event);
        Arc::new(move || {
            // SAFETY: a handle this closure keeps alive.
            unsafe { windows_sys::Win32::System::Threading::SetEvent(event.0) };
        })
    }

    /// Clears the event, so the next wait blocks again.
    pub fn drain(&self) {
        // SAFETY: a handle this struct keeps alive.
        unsafe { windows_sys::Win32::System::Threading::ResetEvent(self.event.0) };
    }
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Blocks on the pipe for at most `timeout`, and says whether it fired.
    fn poll_for(pipe: &WakePipe, timeout: Duration) -> bool {
        use std::os::fd::AsRawFd;
        let mut fds = [libc::pollfd {
            fd: pipe.as_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout.as_millis() as i32) };
        ready > 0
    }

    #[test]
    fn a_worker_thread_can_end_a_block_that_has_no_timeout() {
        // The whole point. Without this the loop would sit on its socket with
        // an answer already in memory until the user happened to move the
        // mouse.
        let pipe = WakePipe::new().expect("a pipe");
        let notify = pipe.notifier();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            notify();
        });

        let began = Instant::now();
        assert!(
            poll_for(&pipe, Duration::from_secs(5)),
            "the poll should have been woken"
        );
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "it waited {:?}",
            began.elapsed()
        );
    }

    #[test]
    fn a_drained_pipe_blocks_again() {
        // The failure this catches is a loop that spins: a pipe left readable
        // makes every subsequent poll return immediately, and an idle desktop
        // becomes a busy one.
        let pipe = WakePipe::new().expect("a pipe");
        pipe.notifier()();
        assert!(poll_for(&pipe, Duration::from_millis(50)));
        pipe.drain();

        let began = Instant::now();
        assert!(!poll_for(&pipe, Duration::from_millis(120)));
        assert!(
            began.elapsed() >= Duration::from_millis(100),
            "an empty pipe should block, but returned after {:?}",
            began.elapsed()
        );
    }

    #[test]
    fn many_wake_ups_are_one_wake_up() {
        // A worker writing while the loop is busy must never block, and a
        // thousand completions must not need a thousand turns. The host drains
        // everything when it wakes, so one byte and a full pipe mean the same
        // thing.
        let pipe = WakePipe::new().expect("a pipe");
        let notify = pipe.notifier();
        let began = Instant::now();
        for _ in 0..100_000 {
            notify();
        }
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "writing to a full pipe blocked: {:?}",
            began.elapsed()
        );
        assert!(poll_for(&pipe, Duration::from_millis(50)));
        pipe.drain();
        assert!(!poll_for(&pipe, Duration::from_millis(50)));
    }
}
