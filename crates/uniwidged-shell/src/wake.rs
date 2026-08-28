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

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;

/// The read end, held by the loop; and a writer, cloned to workers.
pub struct WakePipe {
    read: OwnedFd,
    write: Arc<OwnedFd>,
}

impl WakePipe {
    pub fn new() -> io::Result<Self> {
        let mut fds = [0 as RawFd; 2];
        // SAFETY: `pipe2` writes two descriptors into the array it is given.
        // `O_CLOEXEC` so a widget's command does not inherit them —
        // `Commands` clears the environment but a descriptor is not
        // environment — and `O_NONBLOCK` so a full pipe never stalls a worker.
        let made = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
        if made != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: both are freshly created and owned by nothing else.
        let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        Ok(Self {
            read,
            write: Arc::new(write),
        })
    }

    /// What the loop adds to its `poll` set.
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        // `OwnedFd` borrows as itself; named for what the caller wants.
        use std::os::fd::AsFd;
        self.read.as_fd()
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
        let mut sink = [0u8; 64];
        loop {
            // SAFETY: a valid descriptor and a local buffer of the length
            // given. Non-blocking, so this returns -1/EAGAIN when empty.
            let read =
                unsafe { libc::read(self.read.as_raw_fd(), sink.as_mut_ptr().cast(), sink.len()) };
            if read <= 0 {
                return;
            }
        }
    }
}

#[cfg(test)]
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
