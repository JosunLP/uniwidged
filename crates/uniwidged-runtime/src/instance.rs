//! One widget, running.
//!
//! Everything a widget can reach passes through this file, so it is written to
//! be read rather than to be clever.
//!
//! ## The invariant
//!
//! **The desktop never blocks on guest code.** Every call in is bounded by an
//! epoch deadline, and a widget that blows it is interrupted mid-instruction —
//! not asked politely to stop. That is what makes running other people's code
//! on a desktop reasonable at all, and it is why the deadline is enforced with
//! epochs rather than with fuel: fuel counts instructions, so a guest that
//! blocks on a host call it was granted still hangs. An epoch is wall-clock
//! time, and a background ticker advances it whatever the guest is doing.
//!
//! ## Capabilities are checked here, once
//!
//! Every host function starts by asking [`Policy`]. There is no second path to
//! any of them: WASI Preview 2 grants no ambient authority, so a capability
//! that is not wired up below does not exist to a widget rather than being
//! denied to it.
//!
//! What happens *after* the check is [`Services`] — what this host can
//! actually serve, which is a different question and deliberately a separate
//! type. A capability nobody granted answers `not-granted`; one that was
//! granted and has nothing behind it answers `unavailable`. A widget can act
//! on the difference, and this file is where it is kept.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::capability::{Denied as PolicyDenied, Policy, Request};
use crate::limits::Limits;
use crate::manifest::{ABI_VERSION, Manifest};
use crate::services::{Services, StoreError};

wasmtime::component::bindgen!({
    path: "../../wit",
    world: "widget",
});

use uniwidged::widget::host::Denied;
// The host interface carries types of its own — the asynchronous ones — so it
// gets an alias beside the view vocabulary rather than being reached through a
// path four levels deep at every use.
use uniwidged::widget::host as wit_host;
use uniwidged::widget::types as wit;

/// How often the epoch advances.
///
/// One millisecond is finer than any deadline worth setting and coarse enough
/// that the ticker costs nothing measurable. It bounds how far past its
/// deadline a runaway widget can get, which is the number that matters.
const EPOCH_TICK: Duration = Duration::from_millis(1);

/// The deadline for instantiation itself.
///
/// Instantiating runs the component's own initialisation — allocators,
/// statics, whatever `wit-bindgen` emits — which is legitimately slower than a
/// frame, so it gets its own budget rather than the per-call one. It is still
/// finite: a widget whose module takes two seconds to initialise is broken, and
/// without a deadline here a widget could hang the host before its first call.
///
/// In epoch ticks, so [`EPOCH_TICK`] milliseconds each.
const INSTANTIATE_DEADLINE: u64 = 2_000;

/// Why a widget could not be started or called.
#[derive(Debug)]
pub enum RuntimeError {
    Engine(String),
    /// The component would not load, or imports something this host does not
    /// provide.
    Component(String),
    /// The widget speaks a different ABI. Refused at instantiation rather than
    /// being allowed to fail confusingly later.
    AbiMismatch {
        widget: u32,
        host: u32,
    },
    /// The widget's own `start` said no.
    Start(String),
    /// Guest code trapped, or was interrupted at its deadline.
    Trap(String),
    /// The widget returned a view the host will not accept — see
    /// [`uniwidged_view::wire`].
    BadView(uniwidged_view::wire::TreeError),
    /// Called after the widget trapped. A component instance that has trapped
    /// cannot be entered again — wasmtime says so in as many words — so the
    /// only recovery is a fresh instance, which is the supervisor's decision
    /// and not this type's.
    Dead,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::Engine(why) => write!(f, "engine: {why}"),
            RuntimeError::Component(why) => write!(f, "component: {why}"),
            RuntimeError::AbiMismatch { widget, host } => write!(
                f,
                "this widget was built for ABI {widget}; this host speaks {host}"
            ),
            RuntimeError::Start(why) => write!(f, "the widget refused to start: {why}"),
            RuntimeError::Trap(why) => write!(f, "the widget stopped: {why}"),
            RuntimeError::BadView(why) => write!(f, "the widget's view was refused: {why}"),
            RuntimeError::Dead => write!(
                f,
                "the widget stopped and cannot be resumed; it needs a fresh instance"
            ),
        }
    }
}

impl std::error::Error for RuntimeError {}

/// The engine, shared by every widget in the process.
///
/// Compiling a component is the expensive part and the engine holds the code
/// cache, so one per host rather than one per widget. It also owns the epoch
/// ticker: a single thread advancing a counter for every store at once.
pub struct Runtime {
    engine: Engine,
    linker: Linker<State>,
}

impl Runtime {
    pub fn new() -> Result<Self, RuntimeError> {
        let mut config = Config::new();
        // The deadline mechanism. See the module note on why not fuel.
        config.epoch_interruption(true);
        config.wasm_component_model(true);
        let engine = Engine::new(&config).map_err(|e| RuntimeError::Engine(e.to_string()))?;

        // One ticker for the whole process. It outlives the `Runtime` on
        // purpose: a store whose engine is gone stops being ticked, and a
        // ticker that stopped is a deadline that never fires.
        let ticking = engine.weak();
        std::thread::Builder::new()
            .name("uniwidged-epoch".into())
            .spawn(move || {
                while let Some(engine) = ticking.upgrade() {
                    std::thread::sleep(EPOCH_TICK);
                    engine.increment_epoch();
                }
            })
            .map_err(|e| RuntimeError::Engine(e.to_string()))?;

        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .map_err(|e| RuntimeError::Engine(e.to_string()))?;
        Widget::add_to_linker::<_, wasmtime::component::HasSelf<State>>(&mut linker, |s| s)
            .map_err(|e| RuntimeError::Engine(e.to_string()))?;

        Ok(Self { engine, linker })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Compiles a widget component. Expensive; do it once per widget file.
    pub fn compile(&self, wasm: &[u8]) -> Result<Component, RuntimeError> {
        Component::new(&self.engine, wasm).map_err(|e| RuntimeError::Component(e.to_string()))
    }

    /// Instantiates a compiled component under a policy.
    ///
    /// `services` is what the embedder can serve on this widget's behalf — its
    /// own data directory, this desktop's date conventions. It is per widget,
    /// not per host: the directory is named after the manifest, and handing
    /// two widgets the same one would undo the isolation the whole crate is
    /// about.
    pub fn instantiate(
        &self,
        component: &Component,
        manifest: Arc<Manifest>,
        policy: Policy,
        services: Services,
    ) -> Result<Instance, RuntimeError> {
        let limits = manifest.limits;
        let state = State {
            table: ResourceTable::new(),
            // Nothing inherited: no stdio, no environment, no clocks beyond
            // what WASI's own defaults allow, no preopened directory. A widget
            // reaches the outside world through the host interface below or not
            // at all.
            wasi: WasiCtxBuilder::new().build(),
            store_limits: StoreLimitsBuilder::new()
                .memory_size(limits.memory_bytes)
                // One memory and a handful of tables is what a component needs;
                // more is a widget doing something a widget does not do.
                .instances(8)
                .tables(16)
                .memories(4)
                .build(),
            policy,
            services,
            manifest: manifest.clone(),
            log: Vec::new(),
        };

        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.store_limits);
        // Instantiation runs guest code too, so it is covered from here. `run`
        // replaces this with the per-call deadline before every entry point —
        // a deadline left over from a previous call would fire in the middle
        // of the next one.
        //
        // Note this cannot be "no deadline": wasmtime adds the delta to the
        // current epoch, so `u64::MAX` overflows.
        store.set_epoch_deadline(INSTANTIATE_DEADLINE);

        let widget = Widget::instantiate(&mut store, component, &self.linker)
            .map_err(|e| RuntimeError::Component(e.to_string()))?;

        let mut instance = Instance {
            store,
            widget,
            limits,
            dead: false,
            manifest,
            last_run: Duration::ZERO,
        };

        // The ABI check comes first, before `start` and before anything else.
        // A widget built against a different world may still instantiate and
        // then misbehave in ways that look like a host bug.
        let version = instance.run(|w, s| w.uniwidged_widget_guest().call_abi_version(s))?;
        if version != ABI_VERSION {
            return Err(RuntimeError::AbiMismatch {
                widget: version,
                host: ABI_VERSION,
            });
        }
        match instance.run(|w, s| w.uniwidged_widget_guest().call_start(s))? {
            Ok(()) => {}
            Err(why) => return Err(RuntimeError::Start(why)),
        }
        Ok(instance)
    }
}

/// Everything one running widget owns.
/// One running widget.
///
/// **Single-use.** The first trap — a panic in guest code, or an interruption
/// at the deadline — ends it: a component instance that has trapped cannot be
/// entered again, and wasmtime refuses with "cannot enter component instance".
/// That is not a limitation to work around but the correct behaviour: the
/// widget was stopped mid-function, so its own invariants may be broken and
/// calling further into it would be running code in a state its author never
/// anticipated.
///
/// Recovery is therefore a *fresh instance*, which resets the widget's state —
/// acceptable, because a widget's state is derived from its data. Deciding
/// whether to restart, and how many restarts in a row are one too many, is the
/// supervisor's job; see [`crate::limits::Health`].
pub struct Instance {
    store: Store<State>,
    widget: Widget,
    limits: Limits,
    dead: bool,
    manifest: Arc<Manifest>,
    /// How long the call that ended this instance took, for the supervisor.
    last_run: Duration,
}

impl Instance {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Whether this instance has trapped and can no longer be called.
    ///
    /// A dead widget keeps its surface and its last frame — a crashed widget
    /// leaves a stale panel, not a hole in the desktop — and costs nothing
    /// until the supervisor decides whether to start a new instance.
    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// Whether the call that ended this instance used its whole deadline.
    ///
    /// A widget that hung is worth restarting on a machine that was merely
    /// busy; one that panicked in a millisecond is not.
    pub fn overran_deadline(&self) -> bool {
        self.dead && self.last_run >= self.limits.call_deadline
    }

    /// Anything the widget logged since this was last called.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.data_mut().log)
    }

    /// Runs one call into guest code under a deadline.
    ///
    /// Every entry point goes through here; there is no other way in, which is
    /// what makes the deadline an invariant rather than a convention.
    fn run<T>(
        &mut self,
        call: impl FnOnce(&Widget, &mut Store<State>) -> wasmtime::Result<T>,
    ) -> Result<T, RuntimeError> {
        if self.dead {
            return Err(RuntimeError::Dead);
        }
        // Epochs are milliseconds, so a sub-millisecond deadline would round to
        // zero and interrupt the call before it started.
        let ticks = (self.limits.call_deadline.as_millis() as u64).max(1);
        self.store.set_epoch_deadline(ticks);

        let started = Instant::now();
        match call(&self.widget, &mut self.store) {
            Ok(value) => Ok(value),
            Err(error) => {
                // Any trap ends the instance — see the type's own note. The
                // elapsed time is still worth recording, because it is what
                // tells a supervisor whether this was a widget that hung or a
                // widget that panicked, and only the first is worth retrying
                // on a machine that was merely busy.
                self.dead = true;
                self.last_run = started.elapsed();
                Err(RuntimeError::Trap(error.to_string()))
            }
        }
    }

    /// Asks the widget what should be on screen.
    pub fn view(&mut self) -> Result<uniwidged_view::View, RuntimeError> {
        let tree = self.run(|w, s| w.uniwidged_widget_guest().call_view(s))?;
        from_wire(tree).map_err(RuntimeError::BadView)
    }

    /// Asks when it would like to be called again.
    pub fn next_wake(&mut self) -> Result<uniwidged_core::Wake, RuntimeError> {
        let wake = self.run(|w, s| w.uniwidged_widget_guest().call_next_wake(s))?;
        Ok(match wake {
            wit::Wake::Never => uniwidged_core::Wake::Never,
            wit::Wake::At(seconds) => {
                let epoch = std::time::UNIX_EPOCH;
                match seconds.try_into() {
                    Ok(seconds) => uniwidged_core::Wake::At(epoch + Duration::from_secs(seconds)),
                    // A negative instant is before 1970 and therefore already
                    // past. Treating it as "never" would silently stop a
                    // widget; treating it as due runs it once and lets it ask
                    // again.
                    Err(_) => uniwidged_core::Wake::After(Duration::ZERO),
                }
            }
            wit::Wake::AfterMs(ms) => uniwidged_core::Wake::After(Duration::from_millis(ms as u64)),
            wit::Wake::Animating => uniwidged_core::Wake::Animating,
        })
    }

    /// Delivers an event. `true` means the view would now differ.
    pub fn on_event(&mut self, event: Event) -> Result<bool, RuntimeError> {
        let wit_event = match event {
            Event::Clicked(id) => wit::Event::Clicked(id.0),
            Event::Toggled(id, on) => wit::Event::Toggled((id.0, on)),
            Event::Delivered(token) => wit::Event::Delivered(token),
            Event::Configured => wit::Event::Configured,
            Event::EnvironmentChanged => wit::Event::EnvironmentChanged,
        };
        self.run(|w, s| w.uniwidged_widget_guest().call_on_event(s, wit_event))
    }
}

/// Something that happened to a widget, in host terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Clicked(uniwidged_core::NodeId),
    Toggled(uniwidged_core::NodeId, bool),
    Delivered(u32),
    Configured,
    EnvironmentChanged,
}

/// The store's data: WASI, the limiter, and the policy every host call checks.
pub struct State {
    table: ResourceTable,
    wasi: WasiCtx,
    store_limits: StoreLimits,
    policy: Policy,
    services: Services,
    manifest: Arc<Manifest>,
    log: Vec<String>,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl State {
    /// The one place a capability is checked.
    ///
    /// Returns `()` rather than a value so that every host function below
    /// begins with the same line — `self.check(...)?` — and a function that
    /// forgot to check is visible by its absence.
    fn check(&self, request: Request) -> Result<(), Denied> {
        match self.policy.check(&request) {
            Ok(()) => Ok(()),
            Err(PolicyDenied::NotGranted) => Err(Denied::NotGranted),
            Err(PolicyDenied::Malformed(why)) => Err(Denied::Malformed(why.to_string())),
        }
    }
}

/// The types interface carries no functions; the generated trait is empty and
/// exists only so the world can be linked.
impl uniwidged::widget::types::Host for State {}

impl uniwidged::widget::host::Host for State {
    fn format_time(&mut self, epoch_seconds: i64) -> Result<String, Denied> {
        self.check(Request::Locale)?;
        let locale = self.services.locale.ok_or_else(no_locale)?;
        Ok(locale.format_time(epoch_seconds))
    }

    fn format_date(&mut self, epoch_seconds: i64) -> Result<String, Denied> {
        self.check(Request::Locale)?;
        let locale = self.services.locale.ok_or_else(no_locale)?;
        Ok(locale.format_date(epoch_seconds))
    }

    fn measure_text(&mut self, text: String, role: wit::Role) -> Result<f32, Denied> {
        self.check(Request::Measure)?;
        let metrics = self
            .services
            .measure
            .as_ref()
            .ok_or_else(|| Denied::Unavailable("granted, but this host has no fonts".into()))?;
        Ok(metrics.width(&text, role_from_wire(role)))
    }

    fn fetch(&mut self, url: String) -> Result<u32, Denied> {
        // The host being reached is extracted before the check, because the
        // check is *about* that host — `net` is host-scoped, and a blanket
        // allow is exactly what that exists to prevent.
        let host = host_of(&url).ok_or(Denied::Malformed("not an https URL".into()))?;
        self.check(Request::Net { host })?;
        // The mechanism is here; the transport is not. See `tasks`: a widget
        // granted `net` gets a token as soon as there is something to fetch
        // with, and nothing above this line changes when that lands.
        Err(Denied::Unavailable(NOT_YET.into()))
    }

    fn run_command(&mut self, name: String, args: Vec<String>) -> Result<u32, Denied> {
        self.check(Request::Command { name: name.clone() })?;
        // Bounded here rather than in the runner, because a widget sending ten
        // thousand arguments is a malformed *request* — something its author
        // can fix — and not a command that failed.
        if args.len() > MAX_ARGS {
            return Err(Denied::Malformed(format!(
                "a command may take at most {MAX_ARGS} arguments"
            )));
        }
        if args.iter().map(String::len).sum::<usize>() > MAX_ARG_BYTES {
            return Err(Denied::Malformed(format!(
                "a command's arguments may be at most {MAX_ARG_BYTES} bytes in total"
            )));
        }
        let tasks =
            self.services.tasks.as_ref().ok_or_else(|| {
                Denied::Unavailable("granted, but this host runs no tasks".into())
            })?;
        tasks.run_command(&name, args).map_err(Denied::Unavailable)
    }

    fn take_result(&mut self, token: u32) -> Result<wit_host::TaskResult, Denied> {
        use crate::tasks::TakeError;
        let tasks =
            self.services.tasks.as_ref().ok_or_else(|| {
                Denied::Unavailable("granted, but this host runs no tasks".into())
            })?;
        match tasks.take(token) {
            Ok(result) => Ok(result_to_wire(result)),
            // A token that is not this widget's reads the same as one that
            // never existed — see `tasks`, where the reason is that
            // distinguishing them would tell a widget how many requests its
            // neighbours have made.
            Err(TakeError::NotYours) | Err(TakeError::Gone) => {
                Err(Denied::Malformed("no such request".into()))
            }
            Err(TakeError::NotReady) => Err(Denied::Unavailable(
                "that request has not finished; wait for its `delivered` event".into(),
            )),
        }
    }

    fn read_file(&mut self, path: String) -> Result<Option<Vec<u8>>, Denied> {
        // The path is checked against the policy first and against the
        // filesystem second — see `DataDir`, which does the second check after
        // resolving, because that is the only one that can see a symlink.
        self.check(Request::Fs { path: path.clone() })?;
        let dir = self.services.data.as_ref().ok_or_else(no_data_dir)?;
        dir.read(&path).map_err(|e| match e {
            // Reachable only through a symlink a user planted: the textual
            // check has already passed. Reported as malformed all the same,
            // because the fact about the *request* is the same one.
            StoreError::Escape => Denied::Malformed(ESCAPED.into()),
            other => Denied::Unavailable(other.to_string()),
        })
    }

    fn write_file(&mut self, path: String, bytes: Vec<u8>) -> Result<(), Denied> {
        self.check(Request::Fs { path: path.clone() })?;
        let dir = self.services.data.as_ref().ok_or_else(no_data_dir)?;
        dir.write(&path, &bytes).map_err(|e| match e {
            StoreError::Escape => Denied::Malformed(ESCAPED.into()),
            // A widget asking to store more than the limit is asking for
            // something out of bounds, which is a fact about the request
            // rather than about this host — so the answer is the one it can do
            // something about.
            too_large @ StoreError::TooLarge { .. } => Denied::Malformed(too_large.to_string()),
            other => Denied::Unavailable(other.to_string()),
        })
    }

    fn get_secret(&mut self, _key: String) -> Result<Vec<u8>, Denied> {
        self.check(Request::Secrets)?;
        Err(Denied::Unavailable(NOT_YET.into()))
    }

    fn set_secret(&mut self, _key: String, _value: Vec<u8>) -> Result<(), Denied> {
        self.check(Request::Secrets)?;
        Err(Denied::Unavailable(NOT_YET.into()))
    }

    fn cpu_load(&mut self) -> Result<f32, Denied> {
        self.check(Request::Metrics)?;
        let sampler = self.services.metrics.as_ref().ok_or_else(|| {
            Denied::Unavailable("granted, but this host reads no machine counters".into())
        })?;
        sampler.load().map_err(Denied::Unavailable)
    }

    fn log(&mut self, message: String) {
        // Always available: a widget that cannot say what went wrong is one
        // nobody can help. Bounded, because a widget logging in a loop must
        // not be able to grow the host's memory.
        const MAX_LINES: usize = 256;
        const MAX_LINE: usize = 512;
        if self.log.len() < MAX_LINES {
            let mut message = message;
            message.truncate(MAX_LINE);
            self.log.push(format!("[{}] {message}", self.manifest.name));
        }
    }
}

/// What the capability check passes before the backing implementation exists.
///
/// The gate has been real since M3. `fs`, `locale`, `measure` and `metrics`
/// now have something behind them; `net`, `secrets` and `command` do not yet,
/// and say so. A widget therefore gets `not-granted` when it never asked
/// and `unavailable` when it did — which are different facts, and a widget
/// that conflated them would report the wrong thing to its user.
const NOT_YET: &str = "granted, but this host cannot serve it yet — see ROADMAP.md";

const ESCAPED: &str = "that path leaves the widget's own directory";

/// The most arguments one command may be given, and their total size.
///
/// A widget cannot make the host allocate without bound on its behalf. These
/// are generous for anything a widget legitimately runs and far short of what
/// it takes to matter.
const MAX_ARGS: usize = 64;
const MAX_ARG_BYTES: usize = 16 * 1024;

fn result_to_wire(result: crate::tasks::TaskResult) -> wit_host::TaskResult {
    use crate::tasks::TaskResult;
    match result {
        TaskResult::Command(output) => wit_host::TaskResult::Command(wit_host::CommandOutput {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        }),
        TaskResult::Http(response) => wit_host::TaskResult::Http(wit_host::HttpResponse {
            status: response.status,
            body: response.body,
        }),
        TaskResult::Failed(why) => wit_host::TaskResult::Failed(why),
    }
}

/// Granted, but this embedder gave the widget no directory.
///
/// A headless test and `uniwidged snapshot` both run widgets with nowhere to
/// write, on purpose: rendering a widget should not create files. Saying so
/// plainly is better than inventing a temporary directory the widget would
/// then find empty every time.
fn no_data_dir() -> Denied {
    Denied::Unavailable("granted, but this host gave the widget no data directory".into())
}

fn no_locale() -> Denied {
    Denied::Unavailable("granted, but this host has no locale configured".into())
}

/// The host part of an `https://` URL, lower-cased.
///
/// Anything else — a scheme that is not HTTPS, a URL with no host — returns
/// `None` and is refused. Plain HTTP is not an oversight: a widget's traffic
/// crosses the same networks as everything else, and there is no reason for a
/// new framework to make unencrypted requests possible.
fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest
        .split(['/', '?', '#'])
        .next()?
        .split('@')
        .next_back()?
        .split(':')
        .next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn from_wire(tree: wit::Tree) -> Result<uniwidged_view::View, uniwidged_view::wire::TreeError> {
    use uniwidged_view::wire::Tree;
    Tree {
        nodes: tree.nodes.into_iter().map(node_from_wire).collect(),
    }
    .into_view()
}

fn node_from_wire(node: wit::Node) -> uniwidged_view::wire::WireNode {
    uniwidged_view::wire::WireNode {
        id: node.id,
        content: content_from_wire(node.content),
        style: style_from_wire(node.style),
        semantics: semantics_from_wire(node.semantics),
        children: node.children,
    }
}

fn content_from_wire(content: wit::Content) -> uniwidged_view::Content {
    use uniwidged_view::Content as C;
    match content {
        wit::Content::Group => C::Group,
        wit::Content::Text(text) => C::Text(text),
        wit::Content::Icon(icon) => C::Icon(icon_from_wire(icon)),
        wit::Content::Image(handle) => C::Image(uniwidged_view::ImageHandle(handle)),
        wit::Content::Divider => C::Divider,
        wit::Content::Spacer => C::Spacer,
        wit::Content::Progress(value) => C::Progress(uniwidged_view::Ratio::new(value)),
    }
}

fn icon_from_wire(icon: wit::Icon) -> uniwidged_view::Icon {
    use uniwidged_view::Icon as I;
    match icon {
        wit::Icon::Refresh => I::Refresh,
        wit::Icon::Settings => I::Settings,
        wit::Icon::Close => I::Close,
        wit::Icon::Chevron => I::Chevron,
        wit::Icon::Check => I::Check,
        wit::Icon::Alert => I::Alert,
        wit::Icon::Offline => I::Offline,
    }
}

/// One place, because two would eventually disagree — and a role that decoded
/// differently in `measure-text` than in a view is a widget measuring one
/// thing and being drawn as another.
fn role_from_wire(role: wit::Role) -> uniwidged_view::Role {
    use uniwidged_view::Role;
    match role {
        wit::Role::Plain => Role::Plain,
        wit::Role::Panel => Role::Panel,
        wit::Role::Card => Role::Card,
        wit::Role::Chip => Role::Chip,
        wit::Role::Title => Role::Title,
        wit::Role::Display => Role::Display,
        wit::Role::Section => Role::Section,
        wit::Role::Body => Role::Body,
        wit::Role::BodyStrong => Role::BodyStrong,
        wit::Role::Caption => Role::Caption,
    }
}

fn style_from_wire(style: wit::Style) -> uniwidged_view::Style {
    use uniwidged_view::{Accent, Align, Direction, Justify, Layout};
    uniwidged_view::Style {
        role: role_from_wire(style.role),
        accent: match style.accent {
            wit::Accent::Neutral => Accent::Neutral,
            wit::Accent::Brand => Accent::Brand,
            wit::Accent::Positive => Accent::Positive,
            wit::Accent::Warning => Accent::Warning,
            wit::Accent::Critical => Accent::Critical,
            wit::Accent::Series(index) => Accent::Series(index),
        },
        layout: Layout {
            direction: match style.layout.direction {
                wit::Direction::Column => Direction::Column,
                wit::Direction::Row => Direction::Row,
            },
            gap: style.layout.gap,
            padding: style.layout.padding,
            align: match style.layout.align {
                wit::Align::Start => Align::Start,
                wit::Align::Center => Align::Center,
                wit::Align::End => Align::End,
                wit::Align::Stretch => Align::Stretch,
            },
            justify: match style.layout.justify {
                wit::Justify::Start => Justify::Start,
                wit::Justify::Center => Justify::Center,
                wit::Justify::End => Justify::End,
                wit::Justify::SpaceBetween => Justify::SpaceBetween,
            },
            grow: style.layout.grow,
            wrap: style.layout.wrap,
        },
        muted: style.muted,
    }
}

fn semantics_from_wire(semantics: wit::Semantics) -> uniwidged_view::Semantics {
    use uniwidged_view::{Interaction, State as ViewState};
    uniwidged_view::Semantics {
        label: semantics.label,
        interaction: match semantics.interaction {
            wit::Interaction::None => Interaction::None,
            wit::Interaction::Button => Interaction::Button,
            wit::Interaction::Toggle => Interaction::Toggle,
            wit::Interaction::ListItem => Interaction::ListItem,
            wit::Interaction::Link(url) => Interaction::Link(url),
        },
        state: ViewState {
            disabled: semantics.state.disabled,
            selected: semantics.state.selected,
            checked: semantics.state.checked,
        },
        tooltip: semantics.tooltip,
    }
}
