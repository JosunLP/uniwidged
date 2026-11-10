//! What the host can actually serve.
//!
//! [`Policy`](crate::capability::Policy) decides whether a widget *may* do
//! something; this decides whether the host *can*. The two are deliberately
//! separate, and the separation is visible to a widget: a capability it never
//! declared answers `not-granted`, and one it declared and the user approved
//! but this host has nothing behind answers `unavailable`. A widget that
//! conflated them would send its author to fix a manifest that is already
//! right.
//!
//! So [`Services`] is what an embedder hands the runtime, and every field is
//! optional. A test that supplies nothing gets the same sandbox a desktop
//! does, minus the storage — which is why most of this crate's tests can go on
//! constructing one with [`Services::none`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Everything the host offers on a widget's behalf.
///
/// Cheap to clone: the supervisor keeps one per widget and hands a copy to
/// each fresh instance, because a widget that restarts must find its data
/// where it left it.
///
/// The shared handles are `Arc`, and the types behind them are `Sync`, because
/// Wasmtime requires a store's data to be `Send`. Nothing here actually
/// crosses a thread — one process, one loop, `ARCHITECTURE.md` §5, and the
/// locks are never contended — but a claim the compiler checks is worth more
/// than one it does not.
#[derive(Clone, Default)]
pub struct Services {
    /// The widget's own directory. `None` makes `fs` answer `unavailable`.
    pub data: Option<DataDir>,
    /// How to write a date or a time. `None` makes `locale` answer
    /// `unavailable`.
    pub locale: Option<Locale>,
    /// The host's fonts, for a widget's own truncation decisions.
    pub measure: Option<Arc<dyn TextMetrics>>,
    /// Machine counters. Shared by every widget on the host, because it is one
    /// machine — and because two widgets sampling separately would each see
    /// half the window and report noise.
    pub metrics: Option<Arc<Counters>>,
    /// This widget's access to asynchronous work — `fetch` and `run-command`.
    /// Carries the widget's own id, so no host function can start or collect a
    /// task without saying whose it is.
    pub tasks: Option<crate::tasks::TaskHandle>,
    /// This widget's corner of the keychain. Carries its namespace for the
    /// same reason `tasks` carries its id: there is then no argument through
    /// which a host function could reach somebody else's.
    pub secrets: Option<crate::keyring::Keyring>,
    /// What this widget was configured with.
    ///
    /// Not an `Option`, alone among these, because it is not a capability and
    /// there is nothing for a host to fail to provide: a widget that declared
    /// no settings has an empty one, and every lookup answers `None` — which
    /// is exactly what it means. See `crate::settings`.
    ///
    /// Shared with the running instance rather than copied into it, so a
    /// change reaches a widget that is already started instead of needing it
    /// restarted to alter a number.
    pub settings: crate::settings::Settings,
}

impl std::fmt::Debug for Services {
    /// Hand-written because a measurer is a trait object. It still says
    /// whether one is present, which is the whole of what a reader wants from
    /// this type in a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services")
            .field("data", &self.data)
            .field("locale", &self.locale)
            .field("measure", &self.measure.is_some())
            .field("metrics", &self.metrics.is_some())
            .field("tasks", &self.tasks)
            .field("secrets", &self.secrets)
            .field("settings", &self.settings)
            .finish()
    }
}

impl Services {
    /// A host that can serve nothing. Every granted capability answers
    /// `unavailable`, which is what the conformance suite asserts.
    pub fn none() -> Self {
        Self::default()
    }

    pub fn with_data_dir(mut self, dir: DataDir) -> Self {
        self.data = Some(dir);
        self
    }

    pub fn with_locale(mut self, locale: Locale) -> Self {
        self.locale = Some(locale);
        self
    }

    pub fn with_text_metrics(mut self, metrics: Arc<dyn TextMetrics>) -> Self {
        self.measure = Some(metrics);
        self
    }

    pub fn with_metrics(mut self, sampler: Arc<Counters>) -> Self {
        self.metrics = Some(sampler);
        self
    }

    pub fn with_tasks(mut self, tasks: crate::tasks::TaskHandle) -> Self {
        self.tasks = Some(tasks);
        self
    }

    pub fn with_secrets(mut self, keyring: crate::keyring::Keyring) -> Self {
        self.secrets = Some(keyring);
        self
    }

    pub fn with_settings(mut self, settings: crate::settings::Settings) -> Self {
        self.settings = settings;
        self
    }
}

/// How wide a string is, as the host's own layout would measure it.
///
/// A trait rather than a concrete type because the fonts live two crates away,
/// in `uniwidged-compose`, and the sandbox must not depend on the renderer.
/// The host owns both and joins them.
///
/// The answer has to be the number the compose pass would arrive at — same
/// font, same size, same weight, same digit spacing — or a widget deciding its
/// own truncation is deciding against a guess. `uniwidged-compose` has a test
/// that the two agree.
pub trait TextMetrics: Send + Sync {
    /// Logical pixels. Never negative; zero for an empty string.
    fn width(&self, text: &str, role: uniwidged_view::Role) -> f32;
}

// ---------------------------------------------------------------------------
// The data directory
// ---------------------------------------------------------------------------

/// The most one widget may put in one file, unless the embedder says otherwise.
///
/// Not a disk quota — it is the bound on what one host call can allocate.
/// Without it a widget could ask the host to read a file somebody else put in
/// its directory and make the *host* allocate a gigabyte, which no guest
/// memory cap would catch.
pub const DEFAULT_MAX_FILE_BYTES: usize = 4 * 1024 * 1024;

/// The most one widget may put in its directory altogether.
///
/// The per-file cap above bounds what a single host call can allocate; this
/// bounds what a widget can do with a *loop*. Without it, `fs` is a way to
/// fill somebody's disk one four-megabyte file at a time, which was named as
/// missing in `ROADMAP.md` for exactly as long as it took somebody to write
/// the loop.
///
/// Thirty-two megabytes is far more than any widget's cache and far less than
/// an amount worth noticing on a desktop. An embedder that knows better sets
/// its own with [`DataDir::with_max_total_bytes`]; there is deliberately no
/// per-widget setting in the manifest, because a quota a widget asks for is a
/// quota every widget asks to raise.
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;

/// One widget's own directory on disk.
///
/// The path a widget asks for is relative and stays relative: it is refused
/// unless `capability::is_contained` accepts it, joined onto this root, and
/// then checked *again* after the filesystem has resolved it. The
/// second check is the one that matters, because the first cannot see a
/// symlink — and a widget's directory is somewhere a user can reach with a
/// file manager.
#[derive(Debug, Clone)]
pub struct DataDir {
    root: PathBuf,
    max_file_bytes: usize,
    max_total_bytes: u64,
    /// What the directory was last known to hold, shared by every clone of
    /// this handle so that a widget's restart does not start counting again.
    ///
    /// Because **a write happens inside a widget's `view`, on its
    /// sixteen-millisecond deadline**, and walking a directory to answer "is
    /// there room?" is work proportional to what is already there — charged to
    /// the widget, on every write, for an answer that is almost always yes.
    /// The first version did exactly that, and it is the same mistake the
    /// keychain made with `fsync` and `Tasks::start` made with a thread per
    /// request.
    ///
    /// See [`DataDir::write`] for why a cache is sound here when
    /// [`DataDir::used_bytes`] says a cached total would not be.
    used: Arc<Mutex<Option<u64>>>,
}

/// Why a read or a write did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The path resolved outside the widget's own directory. Only reachable
    /// through a symlink, since the textual check runs first.
    Escape,
    TooLarge {
        limit: usize,
    },
    /// The widget's directory is already as large as it may be. Reported
    /// separately from [`StoreError::TooLarge`] because the two have different
    /// fixes: a widget writing one oversized file has a bug in that write, and
    /// a widget that filled its quota has one in what it keeps.
    Full {
        limit: u64,
        used: u64,
    },
    Io(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Escape => write!(f, "that path leaves the widget's own directory"),
            StoreError::TooLarge { limit } => {
                write!(f, "a widget's file may be at most {limit} bytes")
            }
            StoreError::Full { limit, used } => write!(
                f,
                "a widget may store at most {limit} bytes and this one is \
                 already using {used}"
            ),
            StoreError::Io(why) => write!(f, "{why}"),
        }
    }
}

impl DataDir {
    /// The directory is not created here. A widget that only ever reads should
    /// leave nothing behind, so the first *write* creates it.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            used: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_max_file_bytes(mut self, bytes: usize) -> Self {
        self.max_file_bytes = bytes;
        self
    }

    pub fn with_max_total_bytes(mut self, bytes: u64) -> Self {
        self.max_total_bytes = bytes;
        self
    }

    pub fn max_total_bytes(&self) -> u64 {
        self.max_total_bytes
    }

    /// What this widget is using, in bytes, counting only regular files.
    ///
    /// Always walked: this is the authority, and a cached total would be wrong
    /// after anything outside this process touched the directory — a `--purge`
    /// that did not finish, a user deleting a cache by hand, a restore from a
    /// backup. [`DataDir::write`] caches what this returns and comes back here
    /// whenever the cached answer would be a refusal, so the walk happens when
    /// it decides something rather than on every write.
    ///
    /// Symlinks are **not** followed and their targets are not counted: a
    /// widget cannot make its own quota look smaller by pointing at somebody
    /// else's file, and a user who symlinks something in has not thereby given
    /// the widget a bill for it. The containment checks are what stop the
    /// widget writing *through* one.
    pub fn used_bytes(&self) -> Result<u64, StoreError> {
        fn walk(dir: &Path, total: &mut u64, budget: &mut u32) -> Result<(), StoreError> {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(StoreError::Io(e.to_string())),
            };
            for entry in entries {
                // A directory that grew a file mid-walk is not an error; the
                // next write recounts.
                let Ok(entry) = entry else { continue };
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_symlink() {
                    continue;
                }
                if kind.is_dir() {
                    // A widget can nest directories, and a walk with no floor
                    // is a stack overflow in the desktop process. The depth
                    // budget is generous for a cache and finite, which is the
                    // property that matters.
                    if *budget == 0 {
                        continue;
                    }
                    *budget -= 1;
                    walk(&entry.path(), total, budget)?;
                    *budget += 1;
                } else if let Ok(meta) = entry.metadata() {
                    *total = total.saturating_add(meta.len());
                }
            }
            Ok(())
        }
        let mut total = 0;
        let mut budget = 32u32;
        walk(&self.root, &mut total, &mut budget)?;
        Ok(total)
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn max_file_bytes(&self) -> usize {
        self.max_file_bytes
    }

    /// Reads a file. `Ok(None)` means it is not there.
    ///
    /// Absence is not an error, and saying so in the type is the point: the
    /// first run of a caching widget finds nothing, and that is the normal
    /// case rather than the failure case.
    pub fn read(&self, relative: &str) -> Result<Option<Vec<u8>>, StoreError> {
        if !crate::capability::is_contained(relative) {
            return Err(StoreError::Escape);
        }
        let Some(root) = existing_root(&self.root)? else {
            // Nothing has ever been written, so nothing can be read. Reporting
            // this as I/O would make a widget's first run look broken.
            return Ok(None);
        };
        let path = root.join(relative);
        let resolved = match path.canonicalize() {
            Ok(resolved) => resolved,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(StoreError::Io(e.to_string())),
        };
        if !resolved.starts_with(&root) {
            return Err(StoreError::Escape);
        }
        let length = std::fs::metadata(&resolved)
            .map_err(|e| StoreError::Io(e.to_string()))?
            .len();
        if length > self.max_file_bytes as u64 {
            return Err(StoreError::TooLarge {
                limit: self.max_file_bytes,
            });
        }
        match std::fs::read(&resolved) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StoreError::Io(e.to_string())),
        }
    }

    /// Writes a file, creating the directory and any parents on the way.
    ///
    /// Through a temporary file and a rename, so that a widget interrupted at
    /// its deadline — which happens, by design — cannot leave half a JSON
    /// document where its own next run will try to parse one.
    pub fn write(&self, relative: &str, bytes: &[u8]) -> Result<(), StoreError> {
        if !crate::capability::is_contained(relative) {
            return Err(StoreError::Escape);
        }
        if bytes.len() > self.max_file_bytes {
            return Err(StoreError::TooLarge {
                limit: self.max_file_bytes,
            });
        }
        // The quota is checked against what this write *replaces*, not against
        // the directory as it stands: a widget rewriting the same cache file
        // at its full size must not be refused because the old copy is still
        // there. That is the ordinary case for the widget this exists to
        // bound, so getting it wrong would break the honest widgets and leave
        // the loop working.
        //
        // The count comes from the cache, and the directory is walked only
        // when the cache says the answer might be *no*. That is what keeps a
        // widget under its quota from paying for a directory walk on every
        // write, and it is correct in both directions of drift: a cache that
        // is too high refuses a write that should have been allowed, and the
        // recount below is what then allows it; a cache that is too low —
        // which only something outside this process can cause — lets the quota
        // be exceeded by exactly what that something added, and the next
        // recount takes it back. Neither leaves a widget able to grow without
        // bound, which is the whole job.
        let replacing = self.existing_size(relative);
        let projected = |used: u64| {
            used.saturating_sub(replacing)
                .saturating_add(bytes.len() as u64)
        };
        let mut used = self.known_usage()?;
        if projected(used) > self.max_total_bytes {
            used = self.recount()?;
            if projected(used) > self.max_total_bytes {
                return Err(StoreError::Full {
                    limit: self.max_total_bytes,
                    used,
                });
            }
        }
        let after = projected(used);
        std::fs::create_dir_all(&self.root).map_err(|e| StoreError::Io(e.to_string()))?;
        let root = self
            .root
            .canonicalize()
            .map_err(|e| StoreError::Io(e.to_string()))?;

        let path = root.join(relative);
        let (parent, name) = match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => (parent.to_path_buf(), name.to_os_string()),
            // `is_contained` above refuses everything with no final
            // component, so this is unreachable rather than merely unlikely.
            _ => return Err(StoreError::Escape),
        };
        std::fs::create_dir_all(&parent).map_err(|e| StoreError::Io(e.to_string()))?;
        let parent = parent
            .canonicalize()
            .map_err(|e| StoreError::Io(e.to_string()))?;
        if !parent.starts_with(&root) {
            return Err(StoreError::Escape);
        }

        let mut temporary = parent.join(&name);
        temporary.as_mut_os_string().push(".uniwidged-tmp");
        // A previous interruption may have left one, and writing through a
        // stale symlink is exactly what the checks above exist to prevent.
        let _ = std::fs::remove_file(&temporary);
        std::fs::write(&temporary, bytes).map_err(|e| StoreError::Io(e.to_string()))?;
        std::fs::rename(&temporary, parent.join(&name)).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            StoreError::Io(e.to_string())
        })?;
        // Only after the write landed. A failed write that had already been
        // counted would shrink the widget's apparent room for good.
        *self.used.lock().unwrap_or_else(|e| e.into_inner()) = Some(after);
        Ok(())
    }

    /// The cached usage, walking the directory once if there is none yet.
    fn known_usage(&self) -> Result<u64, StoreError> {
        if let Some(used) = *self.used.lock().unwrap_or_else(|e| e.into_inner()) {
            return Ok(used);
        }
        self.recount()
    }

    /// Walks the directory and remembers the answer.
    fn recount(&self) -> Result<u64, StoreError> {
        let used = self.used_bytes()?;
        *self.used.lock().unwrap_or_else(|e| e.into_inner()) = Some(used);
        Ok(used)
    }
}

impl DataDir {
    /// How large the file this write replaces is, or zero if there is none.
    ///
    /// Deliberately forgiving: anything that cannot be read counts as absent,
    /// which makes the quota check err towards refusing rather than towards
    /// allowing. The containment checks have already run by the time this is
    /// called.
    fn existing_size(&self, relative: &str) -> u64 {
        std::fs::symlink_metadata(self.root.join(relative))
            .ok()
            .filter(|meta| meta.is_file())
            .map(|meta| meta.len())
            .unwrap_or(0)
    }
}

/// The root, resolved, or `None` if it does not exist yet.
fn existing_root(root: &Path) -> Result<Option<PathBuf>, StoreError> {
    match root.canonicalize() {
        Ok(root) => Ok(Some(root)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StoreError::Io(e.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Running a declared command
// ---------------------------------------------------------------------------

/// Runs the one command a widget's manifest declared.
///
/// ## What a user is agreeing to
///
/// `command:notify-send` reads as "run the `notify-send` command", and it means
/// exactly that — but a command can do whatever that command can do, and there
/// is no way for this type to know what that is. `command:sh` is a request for
/// arbitrary code execution wearing a small word. The manifest parser refuses a
/// name containing a path separator, which stops a widget naming a binary
/// inside its own directory; everything past that is the user's judgement, and
/// the consent line is written to make the judgement possible rather than to
/// make it look small.
///
/// ## What this type does guarantee
///
/// * **No shell.** The name is passed to `exec` directly, so nothing in `args`
///   can become a second command. There is no `sh -c` anywhere in this file.
/// * **The granted name, exactly.** Checked by [`crate::Policy`] before this is
///   reached, and the name is never joined with anything.
/// * **Almost no environment.** `PATH` is passed through, because a command has
///   to be findable. Nothing else is: a desktop process's environment routinely
///   holds session tokens, and handing it to a widget's command would be
///   handing them to the widget.
/// * **No standard input.** A command that waits for input would otherwise
///   occupy a worker until its timeout.
/// * **A bound on time and on output.** Both are the desktop's, not the
///   widget's, and both are enforced by killing rather than by asking.
#[derive(Debug, Clone)]
pub struct Commands {
    timeout: Duration,
    max_output: usize,
}

impl Default for Commands {
    fn default() -> Self {
        Self {
            timeout: crate::tasks::COMMAND_TIMEOUT,
            max_output: crate::tasks::MAX_OUTPUT,
        }
    }
}

impl Commands {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_max_output(mut self, bytes: usize) -> Self {
        self.max_output = bytes;
        self
    }

    /// Runs it and waits. **Called on a worker thread, never on the loop.**
    pub fn run(&self, name: &str, args: &[String]) -> crate::tasks::TaskResult {
        use crate::tasks::{CommandOutput, TaskResult};
        use std::process::{Command, Stdio};

        // Defence in depth: the policy has already refused a name that is not
        // exactly a granted one, and the manifest refused a path at parse time.
        // This is the check that holds if either is ever reached another way.
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return TaskResult::Failed(format!("`{name}` is not a command name"));
        }

        let mut command = Command::new(name);
        command
            .args(args)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // The one variable a command cannot do without. `env_clear` first, so
        // this is an allow-list rather than a subtraction from whatever this
        // process happens to be holding.
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }

        let mut child = match command.spawn() {
            Ok(child) => child,
            // The common case by far: the command is not installed. Said
            // plainly, because a widget author reading "No such file or
            // directory" has to guess which file.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return TaskResult::Failed(format!("`{name}` is not installed on this machine"));
            }
            Err(e) => return TaskResult::Failed(format!("could not run `{name}`: {e}")),
        };

        // Output is read on threads of their own. Waiting on the process first
        // and reading afterwards deadlocks the moment a command fills a pipe
        // buffer — which is 64 KB, so it happens on real output rather than
        // only in theory.
        let cap = self.max_output;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let reader = |pipe: Option<std::process::ChildStdout>| {
            std::thread::spawn(move || pipe.map(|pipe| read_capped(pipe, cap)).unwrap_or_default())
        };
        let out = reader(stdout);
        let err = std::thread::spawn(move || {
            stderr
                .map(|pipe| read_capped(pipe, cap))
                .unwrap_or_default()
        });

        let status = match wait_with_timeout(&mut child, self.timeout) {
            Some(status) => status.code().unwrap_or(-1),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                -1
            }
        };
        TaskResult::Command(CommandOutput {
            status,
            stdout: out.join().unwrap_or_default(),
            stderr: err.join().unwrap_or_default(),
        })
    }
}

/// Reads until the end or the cap, whichever comes first.
///
/// Truncated rather than refused: a widget reading the first line of `uptime`
/// should get it even if something upstream has gone mad.
fn read_capped(mut pipe: impl std::io::Read, cap: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    while out.len() < cap {
        match pipe.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let room = cap - out.len();
                out.extend_from_slice(&chunk[..read.min(room)]);
            }
        }
    }
    // Drained to the end even past the cap, so the child is not left blocked on
    // a full pipe and therefore never exiting.
    let mut sink = [0u8; 8192];
    while matches!(pipe.read(&mut sink), Ok(n) if n > 0) {}
    out
}

/// `wait`, but giving up after `timeout`.
///
/// Polled rather than using a platform waiter, because the alternative is
/// `pidfd` on Linux and `kqueue` elsewhere for a bound that is measured in
/// seconds. Ten milliseconds of latency on a process that ran for two seconds
/// is not worth two platform implementations.
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ---------------------------------------------------------------------------
// Machine counters
// ---------------------------------------------------------------------------

/// How busy the machine's CPUs have been.
///
/// ## Why it samples rather than measures
///
/// There is no such thing as instantaneous CPU load: the kernel counts ticks,
/// and a load is the difference between two counts divided by the time between
/// them. So the first call has no previous count to subtract and answers with
/// the average since boot — a real number, and the only honest one available —
/// and every call after it answers for the window since the last sample.
///
/// **It never blocks.** The alternative implementation sleeps 100 ms between
/// two reads, which on this host would mean a widget holding its deadline open
/// inside `view()`. See `instance.rs`: that is the one thing a host function
/// may not do.
///
/// A second call arriving sooner than `MIN_WINDOW` (250 ms) gets the previous answer
/// rather than a new one computed over three milliseconds, which would be
/// noise rather than a measurement.
pub struct Counters {
    last: Mutex<Option<Sample>>,
    /// The network's own sampler, which keeps its own window: a widget asking
    /// for the CPU must not shorten the window the network rate is measured
    /// over, or the other way round.
    network: crate::machine::Network,
}

/// The shortest window worth dividing by.
///
/// Below this the delta is a handful of ticks and the answer swings between 0
/// and 1 depending on where the sample landed. Eight widgets asking in the
/// same turn of the loop all see one measurement, which is also what they
/// should see: it is one machine.
const MIN_WINDOW: Duration = Duration::from_millis(250);

struct Sample {
    at: Instant,
    busy: u64,
    total: u64,
    /// What was reported for this sample, so a repeat within `MIN_WINDOW`
    /// answers consistently rather than recomputing over nothing.
    load: f32,
}

impl Default for Counters {
    fn default() -> Self {
        Self::new()
    }
}

impl Counters {
    pub fn new() -> Self {
        Self {
            last: Mutex::new(None),
            network: crate::machine::Network::new(),
        }
    }

    /// Busy fraction of all CPUs, from 0.0 to 1.0.
    ///
    /// `Err` carries something a user could act on, and is what a platform
    /// with no counters wired up answers.
    pub fn cpu_load(&self) -> Result<f32, String> {
        let (busy, total) = read_cpu_counters()?;
        let now = Instant::now();
        let mut last = self
            .last
            .lock()
            .expect("the sampler lock is never held across a panic");

        let load = match last.as_ref() {
            // Asked again too soon. The previous answer is a measurement; a
            // new one over three milliseconds would not be.
            Some(previous) if now.duration_since(previous.at) < MIN_WINDOW => previous.load,
            Some(previous) => {
                let elapsed = total.saturating_sub(previous.total);
                let working = busy.saturating_sub(previous.busy);
                // A counter that did not advance — possible if the clock tick
                // has not landed — leaves the answer where it was rather than
                // dividing by zero.
                if elapsed == 0 {
                    previous.load
                } else {
                    (working as f32 / elapsed as f32).clamp(0.0, 1.0)
                }
            }
            // The first call. Since boot is the only window there is, and it
            // is a real one.
            None if total > 0 => (busy as f32 / total as f32).clamp(0.0, 1.0),
            None => 0.0,
        };

        *last = Some(Sample {
            at: now,
            busy,
            total,
            load,
        });
        Ok(load)
    }
}

/// `(busy, total)` jiffies across all CPUs.
#[cfg(target_os = "linux")]
fn read_cpu_counters() -> Result<(u64, u64), String> {
    let stat = std::fs::read_to_string("/proc/stat").map_err(|e| format!("/proc/stat: {e}"))?;
    let line = stat
        .lines()
        .next()
        .ok_or_else(|| "/proc/stat is empty".to_string())?;
    parse_cpu_line(line).ok_or_else(|| format!("/proc/stat begins with {line:?}"))
}

#[cfg(any(target_os = "macos", windows))]
fn read_cpu_counters() -> Result<(u64, u64), String> {
    crate::native::cpu()
}

/// Platforms whose counters are not wired up say so rather than answering
/// zero, which a widget would draw as an idle machine.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn read_cpu_counters() -> Result<(u64, u64), String> {
    Err(format!(
        "this host reads no CPU counters on {}",
        std::env::consts::OS
    ))
}

/// The aggregate line of `/proc/stat`, as `(busy, total)`.
///
/// ```text
/// cpu  user nice system idle iowait irq softirq steal guest guest_nice
/// ```
///
/// `idle` and `iowait` are the not-busy fields; everything else counts as
/// work. `guest` and `guest_nice` are **already included** in `user` and
/// `nice` — adding them again is the classic way to report 120 % load on a
/// machine running virtual machines — so the total is taken over the first
/// eight fields only.
fn parse_cpu_line(line: &str) -> Option<(u64, u64)> {
    let mut fields = line.split_ascii_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let values: Vec<u64> = fields
        .take(8)
        .map(|f| f.parse().ok())
        .collect::<Option<_>>()?;
    // user, nice, system, idle — the four every kernel since 2.6 reports. The
    // rest are optional and simply absent on an old or unusual one.
    if values.len() < 4 {
        return None;
    }
    let total: u64 = values.iter().sum();
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    Some((total.saturating_sub(idle), total))
}

impl Counters {
    /// The fraction of physical memory in use, from 0.0 to 1.0.
    ///
    /// No window and no sample kept: memory is a level rather than a rate, so
    /// one read *is* a measurement, where one read of the CPU counters is a
    /// number since boot. "In use" is `MemTotal - MemAvailable` — what the
    /// kernel could not hand to a new allocation without evicting something.
    /// Not `MemTotal - MemFree`, which counts the page cache and on any
    /// machine that has been up for an hour reads 95 % whatever is running.
    pub fn memory_load(&self) -> Result<f32, String> {
        let (used, total) = read_memory_counters()?;
        if total == 0 {
            return Err("this machine reports no memory at all".into());
        }
        Ok((used as f32 / total as f32).clamp(0.0, 1.0))
    }
}

impl Counters {
    /// The machine's battery, or `None` for a machine without one. See
    /// [`crate::machine::battery`].
    pub fn battery(&self) -> Result<Option<crate::machine::Battery>, String> {
        crate::machine::battery()
    }

    /// How full the disk holding the user's home directory is. See
    /// [`crate::machine::disk_load`].
    pub fn disk_load(&self) -> Result<f32, String> {
        crate::machine::disk_load()
    }

    /// Bytes per second since the last call. See [`crate::machine::Network`].
    pub fn network_throughput(&self) -> Result<crate::machine::Throughput, String> {
        self.network.throughput()
    }
}

/// `(used, total)` in kilobytes.
#[cfg(target_os = "linux")]
fn read_memory_counters() -> Result<(u64, u64), String> {
    let meminfo =
        std::fs::read_to_string("/proc/meminfo").map_err(|e| format!("/proc/meminfo: {e}"))?;
    parse_meminfo(&meminfo).ok_or_else(|| "/proc/meminfo has no MemTotal and MemAvailable".into())
}

#[cfg(any(target_os = "macos", windows))]
fn read_memory_counters() -> Result<(u64, u64), String> {
    crate::native::memory()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn read_memory_counters() -> Result<(u64, u64), String> {
    Err(format!(
        "this host reads no memory counters on {}",
        std::env::consts::OS
    ))
}

/// `MemTotal` and `MemAvailable` out of `/proc/meminfo`, as `(used, total)`.
///
/// ```text
/// MemTotal:       16274596 kB
/// MemFree:         1212244 kB
/// MemAvailable:    9021876 kB
/// ```
///
/// `MemAvailable` has been there since Linux 3.14 (2014) and is the kernel's
/// own estimate; before it existed every tool computed its own from `Cached`
/// and `Buffers` and every one of them was wrong in a different way. A kernel
/// without it is answered with a refusal rather than a guess.
fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let mut total = None;
    let mut available = None;
    for line in text.lines() {
        let mut fields = line.split_ascii_whitespace();
        match fields.next()? {
            "MemTotal:" => total = fields.next()?.parse::<u64>().ok(),
            "MemAvailable:" => available = fields.next()?.parse::<u64>().ok(),
            _ => {}
        }
        if total.is_some() && available.is_some() {
            break;
        }
    }
    let (total, available) = (total?, available?);
    Some((total.saturating_sub(available), total))
}

// ---------------------------------------------------------------------------
// Dates and times
// ---------------------------------------------------------------------------

/// Whether a clock reads 13:00 or 1:00 PM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HourCycle {
    H23,
    H12,
}

/// The order a date's parts are written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateOrder {
    /// `2026-08-28`. Also the fallback for an unknown locale, because it is
    /// the one order nobody misreads.
    Ymd,
    /// `28/08/2026`.
    Dmy,
    /// `08/28/2026`.
    Mdy,
}

/// How this desktop writes a date and a time.
///
/// ## Deliberately small
///
/// This is not CLDR and does not pretend to be. It settles the three things
/// that make a formatted date wrong rather than merely unidiomatic — the UTC
/// offset, the hour cycle and the field order — and stops there, in numerals,
/// with no month names and no weekday names in any language.
///
/// The alternative was a full ICU dependency with its locale data, for a
/// framework whose first four widgets show a clock and a number. When a widget
/// needs "Freitag, 28. August", this type is what grows; nothing above it has
/// to change, because a widget never sees the parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale {
    /// Seconds to add to UTC. Fixed at the moment it is determined, so a
    /// desktop that crosses a daylight-saving boundary is told about it the
    /// same way it is told about a theme change — see `Event::EnvironmentChanged`.
    pub utc_offset_seconds: i32,
    pub hour_cycle: HourCycle,
    pub date_order: DateOrder,
}

impl Default for Locale {
    /// UTC, 24-hour, ISO order. What a test wants and what a host falls back
    /// to when the environment says nothing.
    fn default() -> Self {
        Self {
            utc_offset_seconds: 0,
            hour_cycle: HourCycle::H23,
            date_order: DateOrder::Ymd,
        }
    }
}

impl Locale {
    /// What this machine's environment asks for.
    ///
    /// The offset comes from the C library, so it is the one the rest of the
    /// desktop uses, including daylight saving. The conventions come from
    /// `LC_TIME`/`LC_ALL`/`LANG` through the coarse table in
    /// [`Locale::from_lang`].
    pub fn system() -> Self {
        let lang = ["LC_ALL", "LC_TIME", "LANG"]
            .iter()
            .find_map(|name| std::env::var(name).ok())
            .filter(|value| !value.is_empty());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self {
            utc_offset_seconds: system_utc_offset(now),
            ..Self::from_lang(lang.as_deref())
        }
    }

    /// The conventions of a POSIX locale name, at UTC.
    ///
    /// A coarse table by language, with the one region split that matters
    /// often enough to be worth it: `en_US` writes the month first and counts
    /// to twelve, and no other English-speaking region does either.
    /// Everything unrecognised falls back to day-month-year, which is what
    /// most of the world writes — except that `C` and `POSIX`, which mean
    /// "make no assumptions", get ISO order instead.
    pub fn from_lang(lang: Option<&str>) -> Self {
        let Some(lang) = lang else {
            return Self::default();
        };
        // `de_DE.UTF-8@euro` → `de_DE`.
        let tag = lang
            .split(['.', '@'])
            .next()
            .unwrap_or(lang)
            .replace('-', "_");
        if tag == "C" || tag == "POSIX" || tag.is_empty() {
            return Self::default();
        }
        if tag == "en" || tag.starts_with("en_US") {
            return Self {
                hour_cycle: HourCycle::H12,
                date_order: DateOrder::Mdy,
                ..Self::default()
            };
        }
        let language = tag.split('_').next().unwrap_or(&tag);
        let date_order = match language {
            // The languages whose everyday written date is year-first.
            "ja" | "zh" | "ko" | "hu" | "lt" | "sv" => DateOrder::Ymd,
            _ => DateOrder::Dmy,
        };
        Self {
            hour_cycle: HourCycle::H23,
            date_order,
            ..Self::default()
        }
    }

    /// `07:05`, or `7:05 AM`.
    ///
    /// No seconds: a widget showing them asks to be woken sixty times as
    /// often, and `ARCHITECTURE.md` §5 is about what that costs. A widget that
    /// genuinely wants them can format its own.
    pub fn format_time(&self, epoch_seconds: i64) -> String {
        let (_, _, _, hour, minute, _) = self.civil(epoch_seconds);
        match self.hour_cycle {
            HourCycle::H23 => format!("{hour:02}:{minute:02}"),
            HourCycle::H12 => {
                let suffix = if hour < 12 { "AM" } else { "PM" };
                let twelve = match hour % 12 {
                    0 => 12,
                    other => other,
                };
                format!("{twelve}:{minute:02} {suffix}")
            }
        }
    }

    /// `2026-08-28`, `28/08/2026` or `08/28/2026`.
    pub fn format_date(&self, epoch_seconds: i64) -> String {
        let (year, month, day, _, _, _) = self.civil(epoch_seconds);
        match self.date_order {
            DateOrder::Ymd => format!("{year:04}-{month:02}-{day:02}"),
            DateOrder::Dmy => format!("{day:02}/{month:02}/{year:04}"),
            DateOrder::Mdy => format!("{month:02}/{day:02}/{year:04}"),
        }
    }

    /// Year, month, day, hour, minute, second — in this locale's own offset.
    pub fn civil(&self, epoch_seconds: i64) -> (i64, u32, u32, u32, u32, u32) {
        let local = epoch_seconds.saturating_add(self.utc_offset_seconds as i64);
        // Euclidean, not truncating: an instant before 1970 must land on the
        // day before, and `-1 / 86400` is zero.
        let days = local.div_euclid(86_400);
        let seconds = local.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        (
            year,
            month,
            day,
            (seconds / 3600) as u32,
            ((seconds % 3600) / 60) as u32,
            (seconds % 60) as u32,
        )
    }
}

/// Days since 1970-01-01 to a calendar date, proleptic Gregorian.
///
/// Howard Hinnant's `civil_from_days`, which is exact for every day this can
/// be handed and has no branches for leap years — the era arithmetic makes
/// them fall out. Worth using rather than deriving: the obvious version is
/// wrong in 2100, and nobody's test suite runs in 2100.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153; // [0, 11], March-based
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (year + i64::from(month <= 2), month, day)
}

/// This machine's offset from UTC at an instant, in seconds.
///
/// Through the C library rather than by reading `/etc/localtime`: the point is
/// to agree with everything else on the desktop, daylight saving and all, and
/// `localtime_r` is what everything else uses.
#[cfg(unix)]
fn system_utc_offset(epoch_seconds: i64) -> i32 {
    let mut out: libc::tm = unsafe { std::mem::zeroed() };
    // `time_t` is 64-bit here and 32-bit on platforms this does not build for,
    // so the cast is a no-op on the ones that matter and not redundant.
    let time = epoch_seconds as libc::time_t;
    // SAFETY: `localtime_r` writes into a `tm` this function owns and reads a
    // `time_t` passed by value; both outlive the call and neither is aliased.
    // It initialises the zone from `TZ` on first use — glibc and musl both do,
    // and there is no `tzset` in the `libc` crate to call explicitly — so the
    // environment is read once per process. Nothing here writes to the
    // environment, so the `tzset`/`setenv` data race is not reachable.
    let ok = unsafe { !libc::localtime_r(&time, &mut out).is_null() };
    if ok { out.tm_gmtoff as i32 } else { 0 }
}

/// Platforms whose shell is still a milestone get UTC, and say so by being
/// obviously wrong rather than plausibly wrong.
#[cfg(not(unix))]
fn system_utc_offset(_epoch_seconds: i64) -> i32 {
    0
}

#[cfg(test)]
mod tests;
