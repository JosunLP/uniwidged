//! A CPU, memory and disk meter, with a minute of CPU history.
//!
//! The reference widget for `metrics`, and an example of the thing widget
//! authors most often get wrong: **how often to ask**. A load average that
//! updates sixty times a second is not more accurate, it is noisier — the host
//! samples over a window, and asking inside that window returns the previous
//! answer anyway. Two seconds is fast enough to watch a build start and slow
//! enough that a screenful of these costs nothing.
//!
//! It also shows the other half of degrading well: the widget is granted
//! `metrics`, but a host that cannot read counters — or a user who declined —
//! leaves it with nothing to show. It says so, in the same shape, rather than
//! showing a zero that reads as an idle machine.
//!
//! And it is the worked example for **settings**. The two thresholds are the
//! numbers this widget cannot pick for its user: on a laptop 60 % is something
//! to notice, and on a build machine it is Tuesday. They are declared in
//! `widget.toml` with a type and a range, so the host refuses anything else
//! before this code runs — which is why `thresholds` below has no branch for a
//! value that is not a number, and needs none.
//!
//! Since ABI 7 it is also the worked example for a **chart**. The last minute
//! of readings goes under the meter as a `sparkline` — the widget hands over
//! the numbers and nothing else; the line, the fill, the height and the marker
//! are the theme's. That is as far as this framework goes towards a canvas,
//! and the reason it goes no further is the same reason there are no colours
//! here: a screen of charts by different authors should be one chart style.

use uniwidged_guest::prelude::*;

/// How often to ask. See the module note.
const PERIOD: std::time::Duration = std::time::Duration::from_secs(2);

/// How many readings the chart keeps: a minute, at [`PERIOD`].
///
/// Far below the chart's own cap. A widget that hands over more than that
/// gets the recent end and nothing else, but a history that is never longer
/// than it needs to be is one that never costs a `drain`.
const HISTORY: usize = 30;

/// What the widget falls back to if it is running on a host older than its own
/// manifest — the only way `cx.setting` can answer `None` for a declared key.
/// The same numbers the manifest declares, and a test asserts that.
const WARN_AT: f32 = 0.6;
const CRITICAL_AT: f32 = 0.85;

pub struct SystemLoad {
    /// The last reading, if there has been one. `None` renders as "—" rather
    /// than as 0 %, because the two mean opposite things to somebody glancing
    /// at a meter.
    load: Option<f32>,
    /// The readings before it, oldest first, at most [`HISTORY`] of them.
    history: Vec<f32>,
    /// How much of the machine's memory is in use. Under the same grant as
    /// the CPU, and shown the same way — a level rather than a rate, so it
    /// gets a bar and no history.
    memory: Option<f32>,
    /// How full the disk holding the user's home directory is. Since ABI 9,
    /// under the same grant again, and a level like memory: a bar, no history.
    disk: Option<f32>,
    /// Why there is no reading, when there is not.
    refused: Option<String>,
    /// Where the meter turns amber and where it turns red.
    ///
    /// Read once in `start` and again on `Configured`, not on every view: a
    /// host call in a `view` that runs every two seconds is work charged to
    /// this widget's deadline for an answer that changes when a person changes
    /// it and at no other time.
    warn_at: f32,
    critical_at: f32,
}

impl Default for SystemLoad {
    fn default() -> Self {
        Self {
            load: None,
            history: Vec::new(),
            memory: None,
            disk: None,
            refused: None,
            warn_at: WARN_AT,
            critical_at: CRITICAL_AT,
        }
    }
}

impl Widget for SystemLoad {
    fn start(&mut self, cx: &Cx) -> Result<(), String> {
        self.read_settings(cx);
        Ok(())
    }

    fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {
        if !matches!(event, Event::Configured) {
            return false;
        }
        // The event `wit/uniwidged.wit` has carried since the first version.
        // Re-read and say whether it matters: a threshold that moved past the
        // current reading changes the colour of the meter, and one that did
        // not changes nothing worth a frame.
        let accents = |w: &Self| (w.accent(w.load), w.accent(w.memory), w.accent(w.disk));
        let was = accents(self);
        self.read_settings(cx);
        was != accents(self)
    }

    fn view(&mut self, cx: &Cx) -> View {
        match cx.cpu_load() {
            Ok(load) => self.record(load.clamp(0.0, 1.0)),
            Err(denied) => {
                // The last reading is kept: a meter that blanks the moment a
                // host hiccups is worse than one showing a number a few
                // seconds old and saying so.
                self.refused = Some(denied.to_string());
            }
        }
        // Memory under the same grant. Refused the same way, but not reported
        // twice: one reason is the reason.
        if let Ok(memory) = cx.memory_load() {
            self.memory = Some(memory.clamp(0.0, 1.0));
        }
        // And the disk. A host older than ABI 9 would refuse to instantiate
        // this widget at all, so there is no branch here for one that lacks
        // the function — only for one that cannot answer it.
        if let Ok(disk) = cx.disk_load() {
            self.disk = Some(disk.clamp(0.0, 1.0));
        }

        let cpu = self.accent(self.load);
        let memory = self.accent(self.memory);
        let disk = self.accent(self.disk);

        panel(NodeId::named("system-load"))
            .gap(2)
            .child(
                row()
                    .align(Align::Center)
                    .child(text("CPU").role(Role::Section))
                    .child(spacer())
                    .child(
                        chip(reading(self.load))
                            .accent(cpu)
                            .id(NodeId::named("reading")),
                    ),
            )
            .child(
                progress(self.load.unwrap_or(0.0))
                    .accent(cpu)
                    .id(NodeId::named("meter")),
            )
            .child(
                // The minute before the meter, oldest first. Labelled, because
                // a screen reader will otherwise skip it — and what it reads is
                // the latest value, which is what the chart is about.
                sparkline(self.history.iter().copied().chain(self.load))
                    .accent(cpu)
                    .label("CPU over the last minute")
                    .id(NodeId::named("history")),
            )
            .child(
                row()
                    .align(Align::Center)
                    .child(text("Memory").role(Role::Section))
                    .child(spacer())
                    .child(
                        chip(reading(self.memory))
                            .accent(memory)
                            .id(NodeId::named("memory-reading")),
                    ),
            )
            .child(
                progress(self.memory.unwrap_or(0.0))
                    .accent(memory)
                    .id(NodeId::named("memory-meter")),
            )
            .child(
                row()
                    .align(Align::Center)
                    .child(text("Disk").role(Role::Section))
                    .child(spacer())
                    .child(
                        chip(reading(self.disk))
                            .accent(disk)
                            .id(NodeId::named("disk-reading")),
                    ),
            )
            .child(
                progress(self.disk.unwrap_or(0.0))
                    .accent(disk)
                    .id(NodeId::named("disk-meter")),
            )
            .children(self.refused.as_ref().map(|why| {
                text(format!("stale: {why}"))
                    .role(Role::Caption)
                    .accent(Accent::Warning)
                    .id(NodeId::named("why"))
            }))
            .view()
    }

    fn wake(&self) -> Wake {
        Wake::After(PERIOD)
    }
}

/// A percentage, or a dash for a reading that was never taken.
fn reading(value: Option<f32>) -> String {
    match value {
        Some(value) => format!("{}%", (value * 100.0).round() as u32),
        None => "—".to_string(),
    }
}

impl SystemLoad {
    /// Takes a reading, moving the previous one into the history.
    fn record(&mut self, load: f32) {
        if let Some(previous) = self.load.replace(load) {
            self.history.push(previous);
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
        self.refused = None;
    }

    /// Reads both thresholds.
    ///
    /// The `_ =>` arms are only reachable on a host that does not know this
    /// widget's manifest — the host checks the declared type before a widget
    /// sees a value, so a key declared as a number cannot arrive as anything
    /// else. They exist so that an old host degrades to the declared defaults
    /// rather than to zero.
    fn read_settings(&mut self, cx: &Cx) {
        self.warn_at = match cx.setting("warn-at") {
            Some(SettingValue::Number(number)) => number as f32,
            _ => WARN_AT,
        };
        self.critical_at = match cx.setting("critical-at") {
            Some(SettingValue::Number(number)) => number as f32,
            _ => CRITICAL_AT,
        };
    }

    /// Colour by meaning, never by hue — the theme decides what "critical"
    /// looks like against this wallpaper at this contrast setting.
    ///
    /// The same thresholds for both meters. A user who wants memory to go red
    /// somewhere else is a user this widget has not met yet, and a second pair
    /// of settings is cheap to add when they turn up.
    fn accent(&self, value: Option<f32>) -> Accent {
        match value {
            // Ordered so that the stricter test wins, which is what makes a
            // user who set `critical-at` below `warn-at` get the reading they
            // asked for rather than an argument about it.
            Some(value) if value >= self.critical_at => Accent::Critical,
            Some(value) if value >= self.warn_at => Accent::Warning,
            Some(_) => Accent::Positive,
            None => Accent::Neutral,
        }
    }
}

uniwidged_guest::export!(SystemLoad);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meter_with_no_reading_shows_a_dash_rather_than_zero() {
        // Off-target `Cx` refuses everything, which is exactly the state a
        // user who declined `metrics` puts the widget in. Zero would read as
        // an idle machine, which is a different and wrong claim.
        let mut widget = SystemLoad::default();
        let view = widget.view(&Cx::new());
        assert!(widget.load.is_none());
        assert!(widget.memory.is_none());
        assert!(widget.disk.is_none());
        assert!(widget.refused.is_some());
        assert!(format!("{view:?}").contains('—'));
    }

    #[test]
    fn the_accent_says_what_the_number_means() {
        let widget = SystemLoad::default();
        assert_eq!(widget.accent(None), Accent::Neutral);
        assert_eq!(widget.accent(Some(0.1)), Accent::Positive);
        assert_eq!(widget.accent(Some(0.7)), Accent::Warning);
        assert_eq!(widget.accent(Some(0.95)), Accent::Critical);
    }

    #[test]
    fn the_thresholds_move_with_the_settings() {
        // The point of the setting: on a build machine 70 % is Tuesday.
        let widget = SystemLoad {
            warn_at: 0.9,
            critical_at: 0.99,
            ..Default::default()
        };
        assert_eq!(widget.accent(Some(0.7)), Accent::Positive);
        assert_eq!(widget.accent(Some(0.95)), Accent::Warning);
    }

    #[test]
    fn a_host_that_knows_no_settings_gets_the_declared_defaults() {
        // Off-target `Cx::setting` answers `None`, which is what a host older
        // than this widget's manifest looks like. Falling back to zero would
        // paint every reading red.
        let mut widget = SystemLoad {
            warn_at: 0.0,
            critical_at: 0.0,
            ..Default::default()
        };
        widget.read_settings(&Cx::new());
        assert_eq!(widget.warn_at, WARN_AT);
        assert_eq!(widget.critical_at, CRITICAL_AT);
    }

    #[test]
    fn the_fallbacks_are_the_numbers_the_manifest_declares() {
        // Two places state the same default, and a reader has to be able to
        // trust that they agree. The manifest is the authority — it is what
        // the host enforces and what `uniwidged configure` shows — so this
        // reads it rather than restating it.
        let manifest = include_str!("../widget.toml");
        assert!(
            manifest.contains(&format!("default = {WARN_AT}")),
            "widget.toml should declare warn-at = {WARN_AT}"
        );
        assert!(
            manifest.contains(&format!("default = {CRITICAL_AT}")),
            "widget.toml should declare critical-at = {CRITICAL_AT}"
        );
    }

    #[test]
    fn a_configured_event_is_the_only_one_that_re_reads() {
        let mut widget = SystemLoad::default();
        // Nothing else should cost a host call, and none of them should claim
        // the view changed.
        assert!(!widget.on_event(&Event::Clicked(NodeId::named("anything")), &Cx::new()));
        assert!(!widget.on_event(&Event::EnvironmentChanged, &Cx::new()));
        // And a `Configured` that changes nothing is not a frame either.
        assert!(!widget.on_event(&Event::Configured, &Cx::new()));
    }

    #[test]
    fn the_history_is_a_minute_and_ends_with_the_reading_before_this_one() {
        // Oldest first, bounded, and the current reading is not in it — it
        // is drawn as the chart's last point from `load` instead, so the chart
        // and the meter can never disagree about *now*.
        let mut widget = SystemLoad::default();
        for i in 0..(HISTORY + 10) {
            widget.record(i as f32 / 100.0);
        }
        assert_eq!(widget.history.len(), HISTORY);
        assert_eq!(widget.load, Some(0.39));
        assert_eq!(widget.history.last().copied(), Some(0.38));
        assert_eq!(widget.history.first().copied(), Some(0.09));
    }

    #[test]
    fn the_chart_carries_the_history_and_then_the_reading() {
        let mut widget = SystemLoad::default();
        widget.record(0.2);
        widget.record(0.5);
        let view = widget.view(&Cx::new());
        // Off-target the read is refused, so the readings above stand.
        fn find(node: &Node, id: NodeId) -> Option<&Node> {
            if node.id == id {
                return Some(node);
            }
            node.children.iter().find_map(|c| find(c, id))
        }
        let chart = find(&view.root, NodeId::named("history")).expect("the chart");
        let Content::Sparkline(series) = &chart.content else {
            panic!("the history should be a sparkline, got {:?}", chart.content);
        };
        let points: Vec<f32> = series.0.iter().map(|r| r.get()).collect();
        assert_eq!(points, vec![0.2, 0.5]);
        assert_eq!(
            chart.semantics.label.as_deref(),
            Some("CPU over the last minute")
        );
    }
}
