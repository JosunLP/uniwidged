//! A counter that survives a restart.
//!
//! The reference widget for `fs`, and the one that demonstrates the property
//! that makes the whole supervisor design work: **a widget's state is derived
//! from its data**. This one is killed and restarted by the host whenever it
//! misbehaves, and by the user whenever they log out — and it comes back with
//! the same number, because the number is on disk rather than in memory.
//!
//! It also shows the two things a widget with storage has to get right:
//!
//! * **Loading happens in `start`**, which is what `start` is for. Doing it
//!   lazily in `view` behind a `loaded` flag works and is what every widget
//!   ends up writing when `start` cannot reach the host.
//! * **A missing file is not an error.** The first run finds nothing, and that
//!   is the normal case — which is why `read_file` answers `Ok(None)` rather
//!   than something a widget has to read an error string to interpret.

use uniwidged_guest::prelude::*;

/// Inside `state/`, which is the only thing this widget's manifest asks for.
const FILE: &str = "state/count";

#[derive(Default)]
pub struct Counter {
    count: u64,
    /// Set when the count could not be saved. Shown, because a counter that
    /// silently stops remembering is worse than one that says it has.
    unsaved: bool,
}

impl Widget for Counter {
    fn start(&mut self, cx: &Cx) -> Result<(), String> {
        match cx.read_file(FILE) {
            // The first run. Not an error, and not something to tell anybody
            // about.
            Ok(None) => Ok(()),
            Ok(Some(bytes)) => {
                self.count = std::str::from_utf8(&bytes)
                    .ok()
                    .and_then(|text| text.trim().parse().ok())
                    // A file somebody hand-edited into nonsense. Starting from
                    // zero is better than refusing to start: the widget's whole
                    // job is to count, and it can count again.
                    .unwrap_or(0);
                Ok(())
            }
            // Granted and unreadable, or never granted at all. Neither stops
            // the widget — it counts, it just will not remember.
            Err(_) => Ok(()),
        }
    }

    fn view(&mut self, _cx: &Cx) -> View {
        panel(NodeId::named("counter"))
            .gap(2)
            .child(text("Counter").role(Role::Section))
            .child(
                text(self.count.to_string())
                    .role(Role::Display)
                    .id(NodeId::named("count")),
            )
            .child(
                row()
                    .gap(1)
                    .child(
                        chip("Count")
                            .accent(Accent::Brand)
                            .on_click(NodeId::named("increment"))
                            .label("Add one"),
                    )
                    .child(
                        chip("Reset")
                            .on_click(NodeId::named("reset"))
                            .label("Back to zero"),
                    )
                    .child(spacer()),
            )
            .children(self.unsaved.then(|| {
                text("not saved")
                    .role(Role::Caption)
                    .accent(Accent::Warning)
                    .id(NodeId::named("unsaved"))
            }))
            .view()
    }

    fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {
        let Event::Clicked(id) = event else {
            return false;
        };
        if *id == NodeId::named("increment") {
            self.count += 1;
        } else if *id == NodeId::named("reset") {
            self.count = 0;
        } else {
            return false;
        }
        // Written on every change rather than on a timer: the host may stop
        // this widget at any moment, and there is no shutdown hook to flush in.
        self.unsaved = cx
            .write_file(FILE, self.count.to_string().as_bytes())
            .is_err();
        true
    }

    /// Nothing scheduled. A counter changes when it is clicked and at no other
    /// time, and asking to be woken anyway is what makes a desktop full of
    /// widgets cost something.
    fn wake(&self) -> Wake {
        Wake::Never
    }
}

uniwidged_guest::export!(Counter);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicking_counts_and_clicking_reset_does_not() {
        let mut widget = Counter::default();
        let cx = Cx::new();
        assert!(widget.on_event(&Event::Clicked(NodeId::named("increment")), &cx));
        assert!(widget.on_event(&Event::Clicked(NodeId::named("increment")), &cx));
        assert_eq!(widget.count, 2);
        assert!(widget.on_event(&Event::Clicked(NodeId::named("reset")), &cx));
        assert_eq!(widget.count, 0);
    }

    #[test]
    fn a_click_on_something_else_changes_nothing_and_says_so() {
        // Returning `true` when nothing changed costs a tree comparison;
        // returning it on every stray event costs one per event forever.
        let mut widget = Counter::default();
        assert!(!widget.on_event(&Event::Clicked(NodeId::named("elsewhere")), &Cx::new()));
        assert!(!widget.on_event(&Event::Configured, &Cx::new()));
    }

    #[test]
    fn a_widget_that_cannot_save_still_counts_and_says_it_did_not_save() {
        // Off-target `Cx` refuses everything, which is the same state a user
        // who declined `fs` puts the widget in.
        let mut widget = Counter::default();
        widget.on_event(&Event::Clicked(NodeId::named("increment")), &Cx::new());
        assert_eq!(widget.count, 1);
        assert!(widget.unsaved);
    }

    #[test]
    fn starting_with_nothing_stored_starts_from_zero() {
        let mut widget = Counter::default();
        assert!(widget.start(&Cx::new()).is_ok());
        assert_eq!(widget.count, 0);
    }
}
