//! Work a widget asked for and is not waiting on.
//!
//! **The desktop never blocks on guest code, and it must never block on a
//! widget's behalf either.** A widget that could call `run-command` and wait
//! for the process would be a widget holding its deadline open, and the host
//! would interrupt it mid-call — so every request here returns a *token*
//! immediately, the work happens elsewhere, and the answer arrives as a
//! `delivered` event carrying that token back.
//!
//! That is the whole shape, and it is deliberately one shape rather than one
//! per capability. `fetch` and `run-command` both start a task; `take-result`
//! collects any of them. Adding a third kind is a variant, not a mechanism.
//!
//! ## Tokens are owned
//!
//! A token identifies a request *and* who made it. Two widgets running at once
//! draw from the same counter, so nothing stops one from guessing another's
//! token — except that [`Tasks::take`] checks the owner, and a widget asking
//! for a token that is not its own is told the request is malformed rather than
//! handed somebody else's output. That check is the reason this is a shared
//! type with an owner on every entry rather than a per-widget queue: a
//! per-widget queue makes the owner implicit, and an implicit owner is one that
//! is eventually not checked.
//!
//! ## What the host has to do
//!
//! Poll [`Tasks::finished`] on every turn and deliver an event for each token.
//! [`Tasks::wake`] is how the loop learns to turn at all: it fires when a task
//! completes, and the desktop's `wait` is watching it. Without that, a task
//! that finished while the loop was blocked on `None` would sit there until
//! something else happened.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use uniwidged_core::WidgetId;

/// The most output one task may return.
///
/// A command that prints a gigabyte would otherwise be a gigabyte in the
/// desktop's memory, and then a gigabyte across the sandbox boundary. Output
/// past this is truncated rather than the task failing: a widget reading the
/// first line of `uptime` should get it even if something upstream went mad.
pub const MAX_OUTPUT: usize = 256 * 1024;

/// How long a command may run before it is killed.
///
/// Not the widget's call deadline — that is milliseconds, and a process is
/// allowed to take longer than a frame. This is the bound on how long a widget
/// can keep a worker occupied.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

/// The most tasks one widget may have in flight.
///
/// A widget that starts a task on every view and never collects one would
/// otherwise accumulate them until the machine notices. Reaching the limit is
/// refused, not queued: a queue would hide the bug.
pub const MAX_IN_FLIGHT: usize = 16;

/// The most tasks running at once, across the whole desktop.
///
/// A widget cannot spawn a thread, and this is why: eight widgets each allowed
/// sixteen in flight is a hundred and twenty-eight processes, and a desktop
/// that forks a hundred and twenty-eight times because a widget author wrote a
/// loop is a desktop nobody keeps running. Past this a request is refused, and
/// the widget can try again on its next turn.
pub const MAX_CONCURRENT: usize = 8;

/// What a finished command left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// The exit code, or -1 where the process was killed.
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// The answer to one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskResult {
    Command(CommandOutput),
    Http(HttpResponse),
    /// The work did not happen. Distinct from a non-zero exit status, which is
    /// a result: a widget should show "the command failed" differently from
    /// "the command said no".
    Failed(String),
}

/// Why a token could not be collected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeError {
    /// No such token, or one belonging to a different widget. Deliberately the
    /// same answer for both: distinguishing them would tell a widget which of
    /// its neighbours' tokens exist.
    NotYours,
    /// Started and not finished. A widget that waits for the `delivered` event
    /// never sees this.
    NotReady,
    /// Already taken. A token is a one-shot receipt.
    Gone,
}

/// A signal the desktop's event loop can wait on.
///
/// The loop blocks on its window system's socket with no timer at all when
/// nothing is scheduled — that is what makes an idle desktop free. A task
/// finishing on another thread has to be able to end that block, and this is
/// the thing it touches.
///
/// Deliberately not called `Wake`: [`uniwidged_core::Wake`] is a widget saying
/// *when it would like to run again*, which is a different idea, and two types
/// with one name in one project is how somebody eventually matches on the
/// wrong one.
#[derive(Clone, Default)]
pub struct Waker {
    /// Set by a worker, cleared by the host. A flag rather than a count: the
    /// host drains *all* finished tasks when it wakes, so two completions and
    /// one wake-up are the same amount of work.
    flag: Arc<AtomicBool>,
    /// What actually interrupts a blocking wait. Optional because a headless
    /// host has no loop to interrupt and should not be made to own a pipe.
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl std::fmt::Debug for Waker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Waker")
            .field("pending", &self.is_set())
            .field("wired", &self.notify.is_some())
            .finish()
    }
}

impl Waker {
    /// A waker that only sets its flag. What a test and a headless host use.
    pub fn flag_only() -> Self {
        Self::default()
    }

    /// A waker that also interrupts whatever the loop is blocked on.
    ///
    /// `notify` is called from a worker thread, so it has to be the smallest
    /// possible thing — a write to a pipe. It must not take a lock the loop
    /// holds, or a finished task deadlocks the desktop.
    pub fn with_notify(notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            flag: Arc::default(),
            notify: Some(notify),
        }
    }

    fn signal(&self) {
        self.flag.store(true, Ordering::Release);
        if let Some(notify) = &self.notify {
            notify();
        }
    }

    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    /// Reads and clears the flag.
    pub fn take(&self) -> bool {
        self.flag.swap(false, Ordering::AcqRel)
    }
}

/// One request in flight or finished.
struct Task {
    owner: WidgetId,
    /// `None` while the work is running.
    result: Option<TaskResult>,
}

/// What a task should do, once it is running.
///
/// A closure rather than a data description, so that the *runtime* knows
/// nothing about how a command is spawned or how HTTPS is spoken. The host
/// supplies the doing; this supplies the promise that it will not be waited on.
pub type Work = Box<dyn FnOnce() -> TaskResult + Send + 'static>;

/// Every widget's outstanding work.
///
/// Shared by the whole desktop — one counter, one concurrency budget — and safe
/// to hand into the sandbox because every entry names its owner.
pub struct Tasks {
    tasks: Mutex<HashMap<u32, Task>>,
    /// Tokens that finished and have not been reported to the host yet.
    finished: Mutex<Vec<(WidgetId, u32)>>,
    /// Never zero: a token of zero is too easy to produce by accident, and a
    /// widget that sent one would be handed whatever happened to be first.
    next: AtomicU32,
    running: AtomicUsize,
    waker: Waker,
}

impl Default for Tasks {
    fn default() -> Self {
        Self::new(Waker::flag_only())
    }
}

impl Tasks {
    pub fn new(waker: Waker) -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            finished: Mutex::new(Vec::new()),
            next: AtomicU32::new(1),
            running: AtomicUsize::new(0),
            waker,
        }
    }

    pub fn waker(&self) -> &Waker {
        &self.waker
    }

    /// Starts one piece of work and returns its token immediately.
    ///
    /// `Err` is a reason the widget can act on: it has too many requests
    /// outstanding, or the desktop is already doing as much as it will.
    pub fn start(self: &Arc<Self>, owner: WidgetId, work: Work) -> Result<u32, StartError> {
        let mut tasks = self.tasks.lock().expect("the task lock outlives no panic");
        if tasks.values().filter(|t| t.owner == owner).count() >= MAX_IN_FLIGHT {
            return Err(StartError::TooManyForWidget);
        }
        // Claimed before the thread is spawned, so that two widgets asking at
        // once cannot both see the last slot.
        if self
            .running
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |running| {
                (running < MAX_CONCURRENT).then_some(running + 1)
            })
            .is_err()
        {
            return Err(StartError::DesktopBusy);
        }

        let token = self.next.fetch_add(1, Ordering::Relaxed);
        tasks.insert(
            token,
            Task {
                owner,
                result: None,
            },
        );
        drop(tasks);

        let shared = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("uniwidged-task".into())
            .spawn(move || {
                let result = work();
                shared.complete(token, owner, result);
            });
        if spawned.is_err() {
            // The machine would not give us a thread. Undo everything rather
            // than leaving a token that can never finish.
            self.running.fetch_sub(1, Ordering::AcqRel);
            self.tasks
                .lock()
                .expect("the task lock outlives no panic")
                .remove(&token);
            return Err(StartError::DesktopBusy);
        }
        Ok(token)
    }

    fn complete(&self, token: u32, owner: WidgetId, result: TaskResult) {
        self.running.fetch_sub(1, Ordering::AcqRel);
        {
            let mut tasks = self.tasks.lock().expect("the task lock outlives no panic");
            // A widget removed while its task ran leaves nothing to fill in,
            // and nothing to deliver to.
            let Some(task) = tasks.get_mut(&token) else {
                return;
            };
            task.result = Some(result);
        }
        self.finished
            .lock()
            .expect("the finished lock outlives no panic")
            .push((owner, token));
        // Last, and only once the result is readable: the host may be woken
        // and ask for it before this function returns.
        self.waker.signal();
    }

    /// Collects a finished result.
    ///
    /// Checks the owner, which is the whole reason tokens are safe to hand to a
    /// sandbox: they are drawn from one counter, so nothing stops a widget
    /// guessing its neighbour's.
    pub fn take(&self, owner: WidgetId, token: u32) -> Result<TaskResult, TakeError> {
        let mut tasks = self.tasks.lock().expect("the task lock outlives no panic");
        let Some(task) = tasks.get(&token) else {
            return Err(TakeError::Gone);
        };
        if task.owner != owner {
            return Err(TakeError::NotYours);
        }
        if task.result.is_none() {
            return Err(TakeError::NotReady);
        }
        let task = tasks.remove(&token).expect("just found");
        Ok(task.result.expect("just checked"))
    }

    /// Every token that has finished since this was last called, and who owns
    /// it.
    ///
    /// Drained, so a token is reported once. The host turns each into a
    /// `delivered` event and schedules the widget.
    pub fn finished(&self) -> Vec<(WidgetId, u32)> {
        self.waker.take();
        std::mem::take(
            &mut *self
                .finished
                .lock()
                .expect("the finished lock outlives no panic"),
        )
    }

    /// Whether anything is waiting to be delivered.
    ///
    /// The scheduler asks: a desktop with a finished task is not idle, however
    /// little else is scheduled.
    pub fn has_finished(&self) -> bool {
        !self
            .finished
            .lock()
            .expect("the finished lock outlives no panic")
            .is_empty()
    }

    /// Forgets everything a widget owns.
    ///
    /// Called when a widget is removed or suspended. A task already running
    /// still runs — there is no way to un-spawn a process — but its result is
    /// dropped rather than delivered to whatever takes that widget's id next.
    pub fn forget(&self, owner: WidgetId) {
        self.tasks
            .lock()
            .expect("the task lock outlives no panic")
            .retain(|_, task| task.owner != owner);
        self.finished
            .lock()
            .expect("the finished lock outlives no panic")
            .retain(|(that, _)| *that != owner);
    }

    /// How many tasks this widget has outstanding, for diagnostics and tests.
    pub fn in_flight(&self, owner: WidgetId) -> usize {
        self.tasks
            .lock()
            .expect("the task lock outlives no panic")
            .values()
            .filter(|task| task.owner == owner)
            .count()
    }
}

/// Why a request could not be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    /// This widget already has [`MAX_IN_FLIGHT`] requests outstanding.
    TooManyForWidget,
    /// The desktop is already running [`MAX_CONCURRENT`] tasks.
    DesktopBusy,
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::TooManyForWidget => write!(
                f,
                "this widget already has {MAX_IN_FLIGHT} requests outstanding; \
                 collect one before starting another"
            ),
            StartError::DesktopBusy => write!(
                f,
                "the desktop is already running {MAX_CONCURRENT} tasks; try again"
            ),
        }
    }
}

#[cfg(test)]
mod tests;

/// One widget's access to the desktop's asynchronous work.
///
/// The owner is baked in rather than passed at each call, so that no host
/// function can start or collect a task without saying whose it is. That is the
/// difference between an ownership check and an ownership check somebody
/// remembered to write.
#[derive(Clone)]
pub struct TaskHandle {
    owner: WidgetId,
    tasks: Arc<Tasks>,
    /// What actually runs a command. `None` means the capability gate has
    /// something behind it in principle and nothing in practice, which the
    /// widget is told as `unavailable`.
    commands: Option<Arc<crate::services::Commands>>,
}

impl std::fmt::Debug for TaskHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskHandle")
            .field("owner", &self.owner)
            .field("in_flight", &self.tasks.in_flight(self.owner))
            .field("commands", &self.commands.is_some())
            .finish()
    }
}

impl TaskHandle {
    pub fn new(owner: WidgetId, tasks: Arc<Tasks>) -> Self {
        Self {
            owner,
            tasks,
            commands: None,
        }
    }

    pub fn with_commands(mut self, commands: Arc<crate::services::Commands>) -> Self {
        self.commands = Some(commands);
        self
    }

    /// Starts a command. The name has already been checked against the policy.
    pub fn run_command(&self, name: &str, args: Vec<String>) -> Result<u32, String> {
        let commands = self
            .commands
            .clone()
            .ok_or_else(|| "granted, but this host runs no commands".to_string())?;
        let name = name.to_string();
        self.tasks
            .start(self.owner, Box::new(move || commands.run(&name, &args)))
            .map_err(|e| e.to_string())
    }

    pub fn take(&self, token: u32) -> Result<TaskResult, TakeError> {
        self.tasks.take(self.owner, token)
    }
}
