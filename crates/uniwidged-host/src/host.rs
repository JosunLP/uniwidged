//! The daemon.
//!
//! One process, one GPU device, one event loop, N widgets. Everything below is
//! the loop `ARCHITECTURE.md` §5 describes, made concrete:
//!
//! ```text
//! loop {
//!     let timeout = host.next_timeout(Instant::now());   // None ⇒ block
//!     shell.wait(timeout);                               // costs nothing
//!     host.tick(Instant::now(), SystemTime::now());
//! }
//! ```
//!
//! `None` is the common case on an idle desktop, and `None` means the process
//! blocks on its event sources with no timer at all.
//!
//! ## The host does not own windows
//!
//! Surfaces are created by the caller and attached here. That is not squeamish
//! layering: the same host runs a real desktop, a headless test and
//! `uniwidged snapshot`, and only the caller knows which. It also means every
//! test in this file exercises the same code a desktop does, minus the
//! swapchain — which the M0 and M4 spikes show produces the same pixels.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use uniwidged_compose::{Compose, Composed, Compositor, Theme};
use uniwidged_core::{Deadline, NodeId, Point, Size, WidgetId};
use uniwidged_render::{RenderError, Renderer, SurfaceId};
use uniwidged_runtime::capability::Policy;
use uniwidged_runtime::instance::{Runtime, RuntimeError};
use uniwidged_runtime::keyring::{Keyring, SecretStore};
use uniwidged_runtime::manifest::Manifest;
use uniwidged_runtime::net::Http;
use uniwidged_runtime::services::{Commands, CpuSampler, DataDir, Locale, Services, TextMetrics};
use uniwidged_runtime::settings::{Setting, SettingValue, Settings};
use uniwidged_runtime::tasks::{TaskHandle, Tasks, Waker};

use crate::scheduler::Scheduler;

/// How long to wait before giving a failed widget a fresh instance.
///
/// Not zero. A widget that dies immediately would otherwise be restarted as
/// fast as the loop can turn, and three restarts at a 16 ms deadline is fifty
/// milliseconds of a core spent finding out what was already known. Long
/// enough that a dying widget is a background annoyance rather than a load,
/// short enough that a widget which failed once is back before anyone looks.
const RESTART_BACKOFF: std::time::Duration = std::time::Duration::from_millis(200);
use crate::supervisor::{Reconfigured, State, Supervised};

/// What a click landed on.
///
/// Richer than a node id because a link is the one thing a click can be that
/// the *host* cannot finish: opening a browser is launching a process, and the
/// host does not launch processes any more than it owns windows. It decides
/// whether, and the caller — which knows what a browser is on this platform —
/// does it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activated {
    /// Nothing addressable was under the pointer.
    Nothing,
    /// A node was activated, and the widget has been told.
    Node(NodeId),
    /// The same, and the node is a link this widget may open.
    Link { node: NodeId, url: String },
    /// The same, and the node is a link this widget may **not** open — an
    /// ungranted host, or something that is not an https URL at all. The
    /// caller says so; it is a fact its author needs and its user is entitled
    /// to see rather than a click that silently does nothing.
    LinkRefused { node: NodeId, why: String },
}

impl Activated {
    /// The node, whatever kind of activation it was. What a test or a log line
    /// wants.
    pub fn node(&self) -> Option<NodeId> {
        match self {
            Activated::Nothing => None,
            Activated::Node(node)
            | Activated::Link { node, .. }
            | Activated::LinkRefused { node, .. } => Some(*node),
        }
    }
}

/// What one turn of the loop did.
///
/// The soak test watches `submits`: on an idle desktop it must be zero, and
/// everything in `ARCHITECTURE.md` §5 rests on that being true rather than
/// hoped for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickReport {
    /// Widgets that were asked for a view.
    pub ran: Vec<WidgetId>,
    /// Widgets whose view had changed and were redrawn.
    pub redrawn: Vec<WidgetId>,
    /// Widgets that failed this turn and will be restarted.
    pub restarting: Vec<WidgetId>,
    /// Widgets given up on this turn.
    pub suspended: Vec<WidgetId>,
    /// Widgets handed the answer to a request they started, and which said the
    /// answer changes what they would draw.
    pub delivered: Vec<WidgetId>,
    /// `queue.submit` calls. Zero when nothing changed.
    pub submits: usize,
}

impl TickReport {
    pub fn is_quiet(&self) -> bool {
        self.redrawn.is_empty() && self.submits == 0
    }
}

#[derive(Debug)]
pub enum HostError {
    Runtime(RuntimeError),
    Render(RenderError),
    NoSuchWidget(WidgetId),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Runtime(e) => write!(f, "{e}"),
            HostError::Render(e) => write!(f, "{e}"),
            HostError::NoSuchWidget(id) => write!(f, "no widget {id}"),
        }
    }
}

impl std::error::Error for HostError {}

/// Everything that runs a desktop full of widgets.
pub struct Host {
    runtime: Runtime,
    compositor: Compositor,
    renderer: Renderer,
    scheduler: Scheduler,
    widgets: Vec<Supervised>,
    /// The last composition of each widget, kept for hit-testing and for the
    /// accessibility tree. The scene inside it is dropped after presenting;
    /// what stays is small.
    composed: HashMap<WidgetId, Composed>,
    next_id: u64,
    /// Device pixels per logical pixel, and whether to mirror. Per host rather
    /// than per widget: they come from the desktop, and a widget that
    /// disagreed with its neighbours about either would look broken.
    scale: f64,
    rtl: bool,
    /// Where widgets' data directories live, if this host has any. `None` —
    /// the default — makes `fs` answer `unavailable`, which is right for a
    /// test and for `uniwidged snapshot`: rendering a widget should not create
    /// files on somebody's disk.
    data_root: Option<PathBuf>,
    /// Where every widget's secrets go. `None` — the default — makes `secrets`
    /// answer `unavailable`, which is right for a test and for
    /// `uniwidged snapshot`: rendering a widget should not create a keychain
    /// on somebody's disk.
    ///
    /// One store for the desktop, and one namespace per widget cut out of it
    /// by [`Keyring`] — because it is one keychain, and because a store per
    /// widget would be a key file per widget to keep the modes right on.
    keyring: Option<Arc<dyn SecretStore>>,
    /// How this desktop writes dates and times. Defaults to the machine's own,
    /// because a host that cannot format a date is not one anybody wants; a
    /// test that needs a fixed one sets it.
    locale: Locale,
    /// One sampler for the whole desktop. It is one machine, and two widgets
    /// sampling separately would each see half the window and report noise.
    cpu: Arc<CpuSampler>,
    /// Every widget's outstanding asynchronous work, and one concurrency budget
    /// shared between them.
    tasks: Arc<Tasks>,
    commands: Arc<Commands>,
    /// One client for the desktop: one connection pool, one set of trusted
    /// roots, and one place to point at a proxy.
    http: Arc<Http>,
}

/// The compositor's fonts, as the sandbox sees them.
///
/// A newtype only because the trait belongs to `uniwidged-runtime` and the
/// measurer to `uniwidged-compose`, and neither may depend on the other: the
/// sandbox must not know about the renderer, and the renderer must not know
/// about the sandbox. The host owns both, so the host joins them — which is
/// what this crate is for.
struct Fonts(uniwidged_compose::Measurer);

impl TextMetrics for Fonts {
    fn width(&self, text: &str, role: uniwidged_view::Role) -> f32 {
        self.0.width(text, role)
    }
}

impl Host {
    /// `frame` is the refresh interval of the output the widgets are on, for
    /// `Wake::Animating`.
    pub fn new(theme: Theme, frame: std::time::Duration) -> Result<Self, HostError> {
        Self::with_waker(theme, frame, Waker::flag_only())
    }

    /// A host whose finished tasks can interrupt a blocking wait.
    ///
    /// A desktop blocks on its window system's socket with no timer at all when
    /// nothing is scheduled — that is what makes it free — so a task finishing
    /// on a worker thread has to be able to end that block. A headless host has
    /// no block to end and uses [`Waker::flag_only`], which is why this is
    /// separate from [`Host::new`] rather than required by it.
    pub fn with_waker(
        theme: Theme,
        frame: std::time::Duration,
        waker: Waker,
    ) -> Result<Self, HostError> {
        Ok(Self {
            runtime: Runtime::new().map_err(HostError::Runtime)?,
            compositor: Compositor::new(theme),
            renderer: Renderer::new().map_err(HostError::Render)?,
            scheduler: Scheduler::new(frame),
            widgets: Vec::new(),
            composed: HashMap::new(),
            next_id: 1,
            scale: 1.0,
            rtl: false,
            data_root: None,
            keyring: None,
            locale: Locale::system(),
            cpu: Arc::new(CpuSampler::new()),
            tasks: Arc::new(Tasks::new(waker)),
            commands: Arc::new(Commands::new()),
            http: Arc::new(Http::new()),
        })
    }

    /// The renderer, so the caller can create the surfaces it knows how to
    /// create — a swapchain on a desktop, readback in a test.
    pub fn renderer_mut(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    /// Where each widget's own directory goes: `<root>/<widget name>`.
    ///
    /// Set before installing anything. Widgets already running keep the
    /// directory they were started with, because moving a widget's data out
    /// from under it while it is reading is worse than an inconsistency in a
    /// setting nobody changes twice.
    pub fn set_data_root(&mut self, root: impl Into<PathBuf>) {
        self.data_root = Some(root.into());
    }

    /// Where every widget's secrets are kept.
    ///
    /// Set before installing anything, for the same reason as
    /// [`Host::set_data_root`]: moving a widget's keychain out from under it
    /// while it is reading is worse than an inconsistency in a setting nobody
    /// changes twice.
    ///
    /// The store is shared and the namespacing is the host's, so a widget has
    /// no argument through which to name another's secrets — see
    /// `uniwidged_runtime::keyring`.
    pub fn set_keyring(&mut self, store: Arc<dyn SecretStore>) {
        self.keyring = Some(store);
    }

    /// What the keychain behind `secrets` actually is, in one line, or `None`
    /// where there is no keychain.
    ///
    /// `uniwidged doctor` prints it. A file store and a session keychain are
    /// both "secrets are served", and a user deciding whether to let a widget
    /// keep a password is entitled to know which one they have.
    pub fn keyring_description(&self) -> Option<String> {
        self.keyring.as_ref().map(|store| store.describe())
    }

    /// How dates and times are written for every widget on this desktop.
    pub fn set_locale(&mut self, locale: Locale) {
        self.locale = locale;
        // Every widget granted `locale` may now format differently, and none
        // of their views changed — the same shape of problem as a theme change,
        // and the same answer.
        self.renderer.mark_all_dirty();
        for widget in &mut self.widgets {
            widget.forget_last_frame();
        }
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// What one widget is served, by name.
    ///
    /// The directory is named after the manifest, which is why the manifest is
    /// strict about what a name may contain: two widgets sharing a name would
    /// share a directory, and the isolation would be gone without anything
    /// looking wrong.
    fn services_for(&self, id: WidgetId, name: &str, declared: &[Setting]) -> Services {
        Services {
            data: self
                .data_root
                .as_ref()
                .map(|root| DataDir::new(root.join(name))),
            locale: Some(self.locale),
            // The measurer follows this compositor's theme, so a widget that
            // asks how wide its label is gets the answer for the theme it is
            // about to be drawn in rather than the one it started under.
            measure: Some(Arc::new(Fonts(self.compositor.measurer()))),
            metrics: Some(self.cpu.clone()),
            // The handle carries the widget's own id, so no host function can
            // start or collect a task without saying whose it is.
            tasks: Some(
                TaskHandle::new(id, self.tasks.clone())
                    .with_commands(self.commands.clone())
                    .with_http(self.http.clone()),
            ),
            // Namespaced by the widget's name, which is the same thing the
            // data directory is named after and is why the manifest is strict
            // about what a name may contain: two widgets sharing a name would
            // share their secrets.
            secrets: self
                .keyring
                .as_ref()
                .map(|store| Keyring::new(store.clone(), name)),
            // Every widget starts at its own declared defaults, so a widget
            // that reads a setting on its very first frame gets the value its
            // author chose rather than nothing. The user's own values arrive
            // through `configure`, which is also what tells the widget.
            settings: Settings::new(
                declared
                    .iter()
                    .map(|setting| (setting.key.clone(), setting.default.clone()))
                    .collect(),
            ),
        }
    }

    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
    }

    pub fn set_rtl(&mut self, rtl: bool) {
        self.rtl = rtl;
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.compositor.set_theme(theme);
        // Every widget's appearance changed and none of their views did, so the
        // ordinary damage check cannot see it. Forgetting the last frame is
        // what makes the next view count as a change; marking the surfaces
        // dirty alone would achieve nothing, because nothing would compose a
        // new scene to put on them.
        self.renderer.mark_all_dirty();
        for widget in &mut self.widgets {
            widget.forget_last_frame();
        }
    }

    /// Compiles and starts a widget.
    pub fn install(
        &mut self,
        wasm: &[u8],
        manifest: Manifest,
        policy: Policy,
    ) -> Result<WidgetId, HostError> {
        let id = WidgetId(self.next_id);
        self.next_id += 1;
        let services = self.services_for(id, &manifest.name, &manifest.settings);
        let widget = Supervised::start(&self.runtime, id, wasm, manifest, policy, services)
            .map_err(HostError::Runtime)?;
        self.widgets.push(widget);
        self.scheduler.insert(id);
        Ok(id)
    }

    /// Gives a widget somewhere to draw.
    pub fn attach(&mut self, id: WidgetId, surface: SurfaceId) -> Result<(), HostError> {
        self.widget_mut(id)?.surface = Some(surface);
        Ok(())
    }

    /// Creates a readback surface for a widget and attaches it.
    ///
    /// What a test and `uniwidged snapshot` use.
    pub fn attach_readback(&mut self, id: WidgetId, size: Size) -> Result<SurfaceId, HostError> {
        let surface = self
            .renderer
            .add_readback_surface(
                (size.width * self.scale).round() as u32,
                (size.height * self.scale).round() as u32,
            )
            .map_err(HostError::Render)?;
        self.attach(id, surface)?;
        Ok(surface)
    }

    fn index(&self, id: WidgetId) -> Result<usize, HostError> {
        self.widgets
            .iter()
            .position(|w| w.id == id)
            .ok_or(HostError::NoSuchWidget(id))
    }

    fn widget_mut(&mut self, id: WidgetId) -> Result<&mut Supervised, HostError> {
        let index = self.index(id)?;
        Ok(&mut self.widgets[index])
    }

    pub fn widget(&self, id: WidgetId) -> Option<&Supervised> {
        self.widgets.iter().find(|w| w.id == id)
    }

    pub fn widgets(&self) -> &[Supervised] {
        &self.widgets
    }

    /// The last composition of a widget, for hit-testing and accessibility.
    pub fn composed(&self, id: WidgetId) -> Option<&Composed> {
        self.composed.get(&id)
    }

    /// How long to wait. `None` means block — no timer, no polling, no cost.
    pub fn next_timeout(&self, now: Instant) -> Option<std::time::Duration> {
        // A widget that has never been drawn is due immediately, whatever it
        // asked for: its surface is empty until it is.
        if self.widgets.iter().any(|w| self.needs_first_frame(w)) {
            return Some(std::time::Duration::ZERO);
        }
        // A finished task is not idleness. Without this the loop would block on
        // its socket with an answer already sitting in memory.
        if self.tasks.has_finished() {
            return Some(std::time::Duration::ZERO);
        }
        self.scheduler.next_timeout(now)
    }

    fn needs_first_frame(&self, widget: &Supervised) -> bool {
        widget.surface.is_some()
            && widget.last_view().is_none()
            // A widget between instances is on the scheduler with a backoff.
            // Treating it as never-drawn here would make it due immediately
            // and bypass that, which is the whole of the backoff's purpose.
            && matches!(widget.state(), State::Running)
    }

    /// One turn of the loop.
    pub fn tick(&mut self, now: Instant, wall: SystemTime) -> Result<TickReport, HostError> {
        let mut report = TickReport::default();

        // Answers first. A widget whose request finished is told before it is
        // asked for a view, so that the view it produces is the one that has
        // the answer in it — telling it afterwards would show the old frame
        // for one more turn.
        for (owner, token) in self.tasks.finished() {
            let Ok(index) = self.index(owner) else {
                continue;
            };
            if self.widgets[index].deliver(
                &self.runtime,
                uniwidged_runtime::instance::Event::Delivered(token),
            ) {
                report.delivered.push(owner);
            }
        }

        let mut due: Vec<WidgetId> = self.scheduler.run_due(now).due;
        for (owner, _) in report.delivered.iter().map(|id| (*id, ())) {
            if !due.contains(&owner) {
                due.push(owner);
            }
        }
        for widget in &self.widgets {
            if self.needs_first_frame(widget) && !due.contains(&widget.id) {
                due.push(widget.id);
            }
        }
        if due.is_empty() {
            return Ok(report);
        }

        // Composed scenes are collected first and rendered together, so the
        // whole frame is one submit's worth of blits however many widgets
        // ticked.
        let mut scenes: Vec<(SurfaceId, uniwidged_render::Scene)> = Vec::new();
        for id in due {
            let Ok(index) = self.index(id) else { continue };
            report.ran.push(id);

            let Some(changed) = self.widgets[index].view(&self.runtime) else {
                match self.widgets[index].state() {
                    State::Suspended(_) => report.suspended.push(id),
                    _ => report.restarting.push(id),
                }
                continue;
            };
            let Some(surface) = self.widgets[index].surface else {
                // No surface yet: the view is remembered, and it will be drawn
                // as soon as one is attached.
                continue;
            };
            if !changed {
                continue;
            }

            let (width, height) = self.renderer.size(surface).unwrap_or((1, 1));
            let logical = Size::new(width as f64 / self.scale, height as f64 / self.scale);
            let widget = &self.widgets[index];
            let view = widget.last_view().expect("a view was just produced");
            let composed = self.compositor.compose(
                Compose::new(view, id, &widget.name, logical)
                    .scale(self.scale)
                    .rtl(self.rtl),
            );
            self.renderer.mark_dirty(surface);
            scenes.push((surface, composed.scene));
            // The scene is moved out; what is kept is the hit rectangles and
            // the accessibility tree, which are small and are needed until the
            // next frame.
            self.composed.insert(
                id,
                Composed {
                    scene: uniwidged_render::Scene::new(),
                    ..composed
                },
            );
            report.redrawn.push(id);
        }

        if !scenes.is_empty() {
            let batch: Vec<(SurfaceId, &uniwidged_render::Scene)> =
                scenes.iter().map(|(s, scene)| (*s, scene)).collect();
            let stats = self.renderer.render(&batch).map_err(HostError::Render)?;
            report.submits = stats.submits;
        }

        // Rescheduling comes last, and a widget that died while producing its
        // view is *not* asked when it would like to run again.
        //
        // That is not tidiness. Asking would go through the supervisor, which
        // would start a fresh instance to answer — and a fresh instance
        // answering successfully resets the failure count, so a widget that
        // hangs on every single view would be restarted forever and never
        // given up on. It took a test on a deliberately broken widget to
        // notice, because the comment here already claimed what the code did
        // not do.
        for id in &report.ran {
            let Ok(index) = self.index(*id) else { continue };
            if report.suspended.contains(id) {
                self.scheduler.suspend(*id);
                // Nothing will collect this widget's results, and a result
                // delivered to whatever takes its place would be worse than
                // one dropped.
                self.tasks.forget(*id);
                continue;
            }
            if report.restarting.contains(id) {
                self.scheduler.schedule(
                    *id,
                    uniwidged_core::Wake::After(RESTART_BACKOFF),
                    now,
                    wall,
                );
                continue;
            }
            let wake = self.widgets[index].wake(&self.runtime);
            self.scheduler.schedule(*id, wake, now, wall);
        }
        Ok(report)
    }

    /// Which capabilities this host can actually serve, and which it cannot.
    ///
    /// For `uniwidged doctor`, and for the widget author whose widget is
    /// reporting `unavailable` and who has no other way to find out whether
    /// that is their manifest, their user's choice, or this host.
    ///
    /// The unserved half is written out rather than derived, because there is
    /// nothing to derive it from: a capability with no implementation has no
    /// field to be `None`. The conformance suite asserts the same list from the
    /// other side, so the two cannot drift apart silently.
    pub fn capabilities(&self) -> Vec<(&'static str, bool)> {
        let services = self.services_for(WidgetId(0), "doctor", &[]);
        vec![
            // Needs nothing behind it: the scheduler is always there.
            ("timer", true),
            ("locale", services.locale.is_some()),
            ("measure", services.measure.is_some()),
            ("metrics", services.metrics.is_some()),
            ("fs", services.data.is_some()),
            ("net", services.tasks.is_some()),
            ("secrets", services.secrets.is_some()),
            ("command", services.tasks.is_some()),
        ]
    }

    /// Does the node under this point want activations of its own?
    ///
    /// What the caller asks before treating a press as the start of a drag: a
    /// press on a button belongs to the widget, and a press on the panel around
    /// it moves the widget. The answer comes from the same compose pass that
    /// produced the hit rectangles, so it cannot disagree with them.
    ///
    /// A widget with nothing composed yet answers `false`: it has no buttons
    /// on screen, so there is nothing a press could belong to.
    pub fn is_interactive_at(&self, id: WidgetId, point: Point) -> bool {
        self.composed
            .get(&id)
            .and_then(|composed| {
                composed
                    .hit(point)
                    .map(|node| composed.is_interactive(node))
            })
            .unwrap_or(false)
    }

    /// Reports a click at a point in a widget's own logical coordinates.
    ///
    /// Hit-testing happens here, against the rectangles the compose pass
    /// recorded — the widget is told *which node*, never where the pointer
    /// was.
    ///
    /// The widget is told either way, including when the node is a link it may
    /// not open: a widget is entitled to know its own button was pressed, and
    /// what is being withheld is the *opening*, not the news.
    pub fn click(&mut self, id: WidgetId, point: Point) -> Result<Activated, HostError> {
        let Some(node) = self.composed.get(&id).and_then(|c| c.hit(point)) else {
            return Ok(Activated::Nothing);
        };
        let index = self.index(id)?;
        // A widget that says its view would differ is scheduled immediately
        // rather than redrawn here: one place decides what a frame is.
        if self.widgets[index].clicked(&self.runtime, node) {
            self.scheduler.schedule(
                id,
                uniwidged_core::Wake::After(std::time::Duration::ZERO),
                Instant::now(),
                SystemTime::now(),
            );
        }

        let Some(url) = self
            .composed
            .get(&id)
            .and_then(|composed| composed.link(node))
            .map(str::to_owned)
        else {
            return Ok(Activated::Node(node));
        };
        Ok(match self.may_open(index, &url) {
            Ok(()) => Activated::Link { node, url },
            Err(why) => Activated::LinkRefused { node, why },
        })
    }

    /// May this widget's link be opened?
    ///
    /// Two checks, and the order matters. The URL is read **first**, by the
    /// same function `net` reads one with, so a link that is not an `https`
    /// URL is refused as malformed rather than reaching a host comparison that
    /// would have nothing sensible to compare. Then the widget's own
    /// `link:<host>` grant.
    ///
    /// The check happens at the click and not when the view was composed. A
    /// widget can draw whatever it likes; what is gated is the moment the
    /// browser would be launched, which is the only moment that does anything.
    fn may_open(&self, index: usize, url: &str) -> Result<(), String> {
        let Some(host) = uniwidged_runtime::net::host_of(url) else {
            // `file://`, `javascript:`, a custom scheme handler somebody
            // registered. None of them is a web page, and a desktop that
            // handed any of them to the system opener would be letting a
            // widget run whatever that scheme is wired to.
            return Err(format!(
                "`{url}` is not an https URL, and only an https URL is opened"
            ));
        };
        match self.widgets[index]
            .policy()
            .check(&uniwidged_runtime::capability::Request::Link { host: host.clone() })
        {
            Ok(()) => Ok(()),
            Err(_) => Err(format!("this widget was not granted `link:{host}`")),
        }
    }

    /// Applies a widget's configuration and tells it.
    ///
    /// This is what `Event::Configured` was always for. The variant has been
    /// in `wit/uniwidged.wit` since the first version and nothing produced it,
    /// because until settings existed there was nothing to be configured *by*
    /// — the same shape of gap `Event::Delivered` was before the asynchronous
    /// work, and closed the same way.
    ///
    /// The values are resolved against the widget's own manifest, so a caller
    /// cannot hand a widget a key it never declared or a value of the wrong
    /// type. Anything wrong with them comes back rather than being applied or
    /// refused wholesale — see `uniwidged_runtime::settings::Settings::resolve`
    /// on why a stale key is not fatal where an over-grant is.
    ///
    /// A widget that says its view would now differ is scheduled immediately
    /// rather than redrawn here: one place decides what a frame is.
    pub fn configure(
        &mut self,
        id: WidgetId,
        values: &std::collections::BTreeMap<String, SettingValue>,
    ) -> Result<Reconfigured, HostError> {
        let index = self.index(id)?;
        let report = self.widgets[index].configure(&self.runtime, values);
        if report.redraws {
            self.scheduler.schedule(
                id,
                uniwidged_core::Wake::After(std::time::Duration::ZERO),
                Instant::now(),
                SystemTime::now(),
            );
        }
        Ok(report)
    }

    /// Forces one widget to be drawn again, whatever its view says.
    ///
    /// For everything that destroys what is on screen without changing what
    /// should be: an X11 `Expose`, a surface resize, a compositor restart. The
    /// damage check compares view trees and cannot see any of those.
    pub fn redraw(&mut self, id: WidgetId) {
        if let Ok(index) = self.index(id) {
            self.widgets[index].forget_last_frame();
            if let Some(surface) = self.widgets[index].surface {
                self.renderer.mark_dirty(surface);
            }
        }
    }

    /// Delivers an event that did not come from a click.
    ///
    /// A hotkey, a menu item, a finished capability request, a configuration
    /// change — none of those arrive as a pointer, and routing them through
    /// hit-testing would mean inventing a coordinate for something that never
    /// had one.
    pub fn deliver(
        &mut self,
        id: WidgetId,
        event: uniwidged_runtime::instance::Event,
    ) -> Result<(), HostError> {
        let index = self.index(id)?;
        if self.widgets[index].deliver(&self.runtime, event) {
            self.scheduler.schedule(
                id,
                uniwidged_core::Wake::After(std::time::Duration::ZERO),
                Instant::now(),
                SystemTime::now(),
            );
        }
        Ok(())
    }

    /// Every widget's accessibility nodes, for the platform adapter.
    pub fn accessibility(&self) -> Vec<(accesskit::NodeId, accesskit::Node)> {
        self.composed
            .values()
            .flat_map(|c| c.a11y.iter().cloned())
            .collect()
    }

    /// Anything the widgets have logged since this was last called.
    pub fn take_log(&mut self) -> Vec<String> {
        self.widgets.iter_mut().flat_map(|w| w.take_log()).collect()
    }

    /// The deadline the loop is waiting on, for diagnostics.
    pub fn deadline(&self, now: Instant) -> Deadline {
        match self.next_timeout(now) {
            Some(timeout) => Deadline::at(now + timeout),
            None => Deadline::never(),
        }
    }
}

#[cfg(test)]
mod tests;
