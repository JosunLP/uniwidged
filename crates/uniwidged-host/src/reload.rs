//! Reading the configuration again, without stopping.
//!
//! `uniwidged configure` writes a file and a desktop is already running. Until
//! this existed the answer was "it takes effect the next time the desktop
//! starts", which is a sentence people read once and then stop changing
//! settings.
//!
//! ## What can change while a desktop is running, and what cannot
//!
//! **Settings can.** They are values behind a shared handle, and
//! [`Host::configure`] already reaches a widget that is already started —
//! which is why it was built that way rather than by restarting the widget.
//!
//! **Placement can**, but not here: the host does not own windows, so it
//! cannot move one. What this reports is which widgets are in the file, and the
//! caller — which does own them — re-resolves and moves. The split is the same
//! one `ARCHITECTURE.md` §5 makes about surfaces, and for the same reason: a
//! headless test and a real desktop run the same host.
//!
//! **Installing and removing cannot**, and that is named rather than faked. A
//! widget arriving needs a component compiled, a window created, a swapchain
//! attached and a placement resolved; a widget leaving needs all of that undone
//! in the right order. Doing half of it on a signal is how a desktop ends up
//! with a window nothing draws into. So a widget that appeared or disappeared
//! is *reported*, the user is told it takes a restart, and the ones that are
//! running keep working.
//!
//! ## Why a diff rather than a rebuild
//!
//! Because a widget that did not change must not notice. Rebuilding every
//! widget's configuration on every reload would deliver `Configured` to all of
//! them, and a widget that redraws on that event would flicker every time
//! somebody changed a different widget's setting.

use std::collections::BTreeSet;

use uniwidged_core::WidgetId;

use crate::config::Config;
use crate::host::Host;

/// What a reload did, and what it could not do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Widgets whose settings actually changed, and which have been told.
    ///
    /// Not the same as "and which said their view now differs", which is what
    /// this listed until a live desktop showed the difference: a widget can be
    /// given a genuinely new number and correctly answer that nothing it draws
    /// depends on it. Reporting only the redrawing ones made a reload say
    /// "nothing changed" over a configuration it had just applied. The ones
    /// that do redraw are already scheduled, so a caller needs no second list.
    pub reconfigured: Vec<WidgetId>,
    /// Named in the file, enabled, and not running. Needs a restart.
    pub added: Vec<String>,
    /// Running, and either gone from the file or disabled in it. Needs a
    /// restart.
    pub removed: Vec<String>,
    /// What was wrong with the file — a key a widget does not declare, a value
    /// it would not accept. Reported rather than fatal, for the reason
    /// `settings::Settings::resolve` gives.
    pub complaints: Vec<String>,
}

impl Applied {
    /// Whether anything changed that the user should be told takes a restart.
    pub fn needs_restart(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty()
    }

    pub fn is_quiet(&self) -> bool {
        self.reconfigured.is_empty() && self.complaints.is_empty() && !self.needs_restart()
    }
}

/// Applies what a configuration file says to the widgets already running.
///
/// A widget is matched by **name**, which is the same identifier its data
/// directory and its keychain namespace use — and the reason the manifest is
/// strict about what a name may contain. An id would not do: ids are this
/// process's, and the file outlives the process.
pub fn apply(host: &mut Host, config: &Config) -> Applied {
    let mut applied = Applied::default();

    let running: Vec<(WidgetId, String)> = host
        .widgets()
        .iter()
        .map(|widget| (widget.id, widget.name.clone()))
        .collect();
    let mut seen = BTreeSet::new();

    for (id, name) in running {
        seen.insert(name.clone());
        let Some(entry) = config.widget(&name) else {
            applied.removed.push(name);
            continue;
        };
        if !entry.enabled {
            applied.removed.push(name);
            continue;
        }

        // Resolved against the *running* widget's manifest rather than
        // against the file's idea of where the manifest is. The two can differ
        // — somebody may have edited the file to point somewhere else — and
        // what a running widget will accept is decided by what it was started
        // with.
        let Some(widget) = host.widget(id) else {
            continue;
        };
        let (values, complaints) = entry.settings(widget.manifest());
        for complaint in complaints {
            applied.complaints.push(format!("`{name}`: {complaint}"));
        }
        match host.configure(id, &values) {
            Ok(report) => {
                if report.applied {
                    applied.reconfigured.push(id);
                }
            }
            // Only reachable if the widget went away between the two lines
            // above, which one thread cannot do.
            Err(e) => applied.complaints.push(format!("`{name}`: {e}")),
        }
    }

    for entry in &config.widgets {
        if entry.enabled && !seen.contains(&entry.name) {
            applied.added.push(entry.name.clone());
        }
    }
    applied
}

#[cfg(test)]
mod tests;
