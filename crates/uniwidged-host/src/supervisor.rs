//! One installed widget, kept running.
//!
//! `uniwidged_runtime::Instance` is single-use: the first trap ends it, because
//! a component instance that has trapped cannot be entered again and should not
//! be — it was stopped mid-function, so its own invariants may be broken. This
//! is what sits above that and decides whether to start another.
//!
//! ## The restart rule
//!
//! One failure is a slow machine under load; a widget that dies again on a
//! fresh instance has a bug, and restarting a bug sixty times a second is how
//! a widget framework becomes the reason a laptop is warm. So
//! [`Health`] counts consecutive failed *instances*, and after
//! [`Limits::overruns_before_suspend`] the widget is given up on until someone
//! asks for it back.
//!
//! Throughout, **the surface keeps its last frame**: a crashed widget leaves a
//! stale panel on the desktop, not a hole in it.

use std::sync::Arc;

use uniwidged_core::{NodeId, Wake, WidgetId};
use uniwidged_render::SurfaceId;
use uniwidged_runtime::capability::Policy;
use uniwidged_runtime::instance::{Event, Instance, Runtime, RuntimeError};
use uniwidged_runtime::limits::{Health, Limits, Outcome};
use uniwidged_runtime::manifest::Manifest;
use uniwidged_runtime::services::Services;
use uniwidged_view::View;
use wasmtime::component::Component;

/// What a widget is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Running,
    /// Failed once and awaiting a fresh instance, which happens on its next
    /// turn rather than immediately — a widget that dies in a loop should not
    /// be able to spin the host by dying quickly.
    Restarting,
    /// Given up on. Shown as errored, still on screen, never run again until
    /// the user asks.
    Suspended(String),
    /// Turned off in the configuration.
    Disabled,
}

/// One installed widget and everything the host knows about it.
pub struct Supervised {
    pub id: WidgetId,
    pub name: String,
    manifest: Arc<Manifest>,
    component: Component,
    policy: Policy,
    /// Handed to every fresh instance, so a widget that restarts finds its
    /// data where it left it. That is what makes a restart cheap: a widget's
    /// in-memory state is derived from its data, and the data survives.
    services: Services,
    instance: Option<Instance>,
    health: Health,
    limits: Limits,
    /// The last tree the widget returned, for the damage check. `None` means
    /// nothing has been drawn yet, so the next view is always a frame.
    last_view: Option<View>,
    /// Where it draws. Attached by the caller, because only the caller knows
    /// whether this desktop has windows or is a test.
    pub surface: Option<SurfaceId>,
    state: State,
}

impl Supervised {
    /// Compiles the component and starts the first instance.
    pub fn start(
        runtime: &Runtime,
        id: WidgetId,
        wasm: &[u8],
        manifest: Manifest,
        policy: Policy,
        services: Services,
    ) -> Result<Self, RuntimeError> {
        let component = runtime.compile(wasm)?;
        let manifest = Arc::new(manifest);
        let limits = manifest.limits;
        let instance = runtime.instantiate(
            &component,
            manifest.clone(),
            policy.clone(),
            services.clone(),
        )?;
        Ok(Self {
            id,
            name: manifest.name.clone(),
            manifest,
            component,
            policy,
            services,
            instance: Some(instance),
            health: Health::default(),
            limits,
            last_view: None,
            surface: None,
            state: State::Running,
        })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// What the user granted this widget.
    ///
    /// A subset of what its manifest declares — see `config`, where granting
    /// *less* is how a user says no to part of a widget without saying no to
    /// all of it. Every fresh instance is handed this same policy, so what a
    /// widget may do does not change when it restarts.
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn is_running(&self) -> bool {
        self.state == State::Running
    }

    /// Turns a widget off without forgetting it.
    pub fn disable(&mut self) {
        self.instance = None;
        self.state = State::Disabled;
    }

    /// Starts a widget the user asked for again, from a clean slate.
    ///
    /// Clears the failure count as well as the instance: a widget somebody
    /// deliberately restarted deserves the same three chances as one that has
    /// never failed.
    pub fn revive(&mut self, runtime: &Runtime) -> Result<(), RuntimeError> {
        self.health = Health::default();
        self.state = State::Restarting;
        self.instance = None;
        self.ensure_instance(runtime)
    }

    fn ensure_instance(&mut self, runtime: &Runtime) -> Result<(), RuntimeError> {
        if self.instance.is_some() {
            return Ok(());
        }
        match runtime.instantiate(
            &self.component,
            self.manifest.clone(),
            self.policy.clone(),
            self.services.clone(),
        ) {
            Ok(instance) => {
                self.instance = Some(instance);
                self.state = State::Running;
                Ok(())
            }
            Err(error) => {
                // A widget that will not even instantiate is not going to
                // instantiate on the next tick either, so this counts as a
                // failure like any other rather than being retried forever.
                self.record_failure(error.to_string());
                Err(RuntimeError::Dead)
            }
        }
    }

    fn record_failure(&mut self, why: String) {
        self.instance = None;
        match self.health.failed(&self.limits) {
            Outcome::Suspended => self.state = State::Suspended(why),
            _ => self.state = State::Restarting,
        }
    }

    /// Runs one call, restarting the widget first if it is between instances.
    ///
    /// Every entry point below goes through this, so the restart rule cannot
    /// be forgotten in one of them.
    fn call<T>(
        &mut self,
        runtime: &Runtime,
        f: impl FnOnce(&mut Instance) -> Result<T, RuntimeError>,
    ) -> Option<T> {
        if matches!(self.state, State::Suspended(_) | State::Disabled) {
            return None;
        }
        if self.ensure_instance(runtime).is_err() {
            return None;
        }
        let instance = self.instance.as_mut()?;
        match f(instance) {
            Ok(value) => {
                self.health.completed();
                Some(value)
            }
            Err(error) => {
                self.record_failure(error.to_string());
                None
            }
        }
    }

    /// Asks the widget for its view. `true` means it differs from the last one.
    ///
    /// The view itself is kept rather than returned, and read back through
    /// [`Supervised::last_view`]. That is not indirection for its own sake: a
    /// caller holding a borrow of the view cannot touch anything else about the
    /// widget while it composes, and composing is exactly when it wants to.
    ///
    /// `None` means the widget produced nothing this turn — suspended, or it
    /// died trying. The caller then leaves the surface alone, which is what
    /// keeps a crashed widget's last frame on screen.
    pub fn view(&mut self, runtime: &Runtime) -> Option<bool> {
        let view = self.call(runtime, |instance| instance.view())?;
        let changed = self
            .last_view
            .as_ref()
            .is_none_or(|previous| view.differs_from(previous));
        self.last_view = Some(view);
        Some(changed)
    }

    /// The last view the widget produced, without asking it again.
    pub fn last_view(&self) -> Option<&View> {
        self.last_view.as_ref()
    }

    /// Forgets what is on screen, so the next view counts as a change.
    ///
    /// For everything that alters a widget's *appearance* without altering its
    /// view: a theme change, a scale change, a resize. The damage check
    /// compares view trees, and none of those touch one — so without this a
    /// light/dark switch would leave every widget in the old theme until it
    /// happened to update on its own.
    pub fn forget_last_frame(&mut self) {
        self.last_view = None;
    }

    pub fn wake(&mut self, runtime: &Runtime) -> Wake {
        // A widget that cannot be asked schedules nothing, which is also what
        // stops a suspended one from costing anything.
        self.call(runtime, |instance| instance.next_wake())
            .unwrap_or(Wake::Never)
    }

    /// Delivers an event. `true` means the widget says its view would differ.
    pub fn deliver(&mut self, runtime: &Runtime, event: Event) -> bool {
        self.call(runtime, |instance| instance.on_event(event))
            .unwrap_or(false)
    }

    /// Reports a click on a node by id, which is how the host reports every
    /// click — never by coordinate.
    pub fn clicked(&mut self, runtime: &Runtime, node: NodeId) -> bool {
        self.deliver(runtime, Event::Clicked(node))
    }

    /// What this widget declared it can be configured with.
    fn declared_settings(&self) -> &[uniwidged_runtime::settings::Setting] {
        &self.manifest.settings
    }

    /// Replaces this widget's configuration and tells it.
    ///
    /// The values are resolved against the widget's own manifest first, so a
    /// caller cannot hand a widget a key it never declared or a value of the
    /// wrong type — the guarantee a widget's `setting` call depends on is
    /// enforced here rather than trusted.
    ///
    /// The handle is *shared* with the running instance, so this reaches a
    /// widget that is already started. Restarting one to change a number would
    /// throw away everything it had loaded.
    ///
    /// Returns whether anything was applied, whether the widget says its view
    /// would now differ, and anything wrong with what it was given.
    ///
    /// The first two are **different facts**, and running this against a real
    /// desktop is what showed why. A widget can be given a genuinely new value
    /// and answer that its view is unchanged — `system-load` moving a warning
    /// threshold past nothing does exactly that — and a caller that read the
    /// second as the first reported "nothing changed" over a configuration it
    /// had just applied.
    pub fn configure(
        &mut self,
        runtime: &Runtime,
        values: &std::collections::BTreeMap<String, uniwidged_runtime::settings::SettingValue>,
    ) -> Reconfigured {
        let (resolved, complaints) =
            uniwidged_runtime::settings::Settings::resolve(self.declared_settings(), values);
        // Unchanged is not an event. A configuration written back with nothing
        // altered — which is what saving a form usually is — should not cost
        // every widget a call and a tree comparison.
        if resolved == self.services.settings.snapshot() {
            return Reconfigured {
                applied: false,
                redraws: false,
                complaints,
            };
        }
        self.services.settings.replace(resolved);
        Reconfigured {
            applied: true,
            redraws: self.deliver(runtime, Event::Configured),
            complaints,
        }
    }

    /// The bitmaps the running instance has registered, or none where there
    /// is no instance — a suspended widget's last frame still shows, and it
    /// was composed while the pixels were there.
    pub fn images(&self) -> Option<&uniwidged_runtime::images::Images> {
        self.instance.as_ref().map(|instance| instance.images())
    }

    /// Anything the widget logged, prefixed with its name.
    pub fn take_log(&mut self) -> Vec<String> {
        self.instance
            .as_mut()
            .map(|instance| instance.take_log())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;

/// What configuring one widget did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconfigured {
    /// The values differed from what this widget was running with, and it has
    /// been told.
    pub applied: bool,
    /// It said its view would now differ. Distinct from `applied`: a widget
    /// can be given a new number and correctly answer that nothing it draws
    /// depends on it.
    pub redraws: bool,
    /// A key it does not declare, a value it would not accept.
    pub complaints: Vec<String>,
}
