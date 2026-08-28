//! A CPU meter.
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

use uniwidged_guest::prelude::*;

/// How often to ask. See the module note.
const PERIOD: std::time::Duration = std::time::Duration::from_secs(2);

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
        let was = self.accent();
        self.read_settings(cx);
        was != self.accent()
    }

    fn view(&mut self, cx: &Cx) -> View {
        match cx.cpu_load() {
            Ok(load) => {
                self.load = Some(load.clamp(0.0, 1.0));
                self.refused = None;
            }
            Err(denied) => {
                // The last reading is kept: a meter that blanks the moment a
                // host hiccups is worse than one showing a number a few
                // seconds old and saying so.
                self.refused = Some(denied.to_string());
            }
        }

        let reading = match self.load {
            Some(load) => format!("{}%", (load * 100.0).round() as u32),
            None => "—".to_string(),
        };

        panel(NodeId::named("system-load"))
            .gap(2)
            .child(
                row()
                    .align(Align::Center)
                    .child(text("CPU").role(Role::Section))
                    .child(spacer())
                    .child(
                        chip(reading)
                            .accent(self.accent())
                            .id(NodeId::named("reading")),
                    ),
            )
            .child(
                progress(self.load.unwrap_or(0.0))
                    .accent(self.accent())
                    .id(NodeId::named("meter")),
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

impl SystemLoad {
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
    fn accent(&self) -> Accent {
        match self.load {
            // Ordered so that the stricter test wins, which is what makes a
            // user who set `critical-at` below `warn-at` get the reading they
            // asked for rather than an argument about it.
            Some(load) if load >= self.critical_at => Accent::Critical,
            Some(load) if load >= self.warn_at => Accent::Warning,
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
        assert!(widget.refused.is_some());
        assert!(format!("{view:?}").contains('—'));
    }

    #[test]
    fn the_accent_says_what_the_number_means() {
        let mut widget = SystemLoad::default();
        assert_eq!(widget.accent(), Accent::Neutral);
        widget.load = Some(0.1);
        assert_eq!(widget.accent(), Accent::Positive);
        widget.load = Some(0.7);
        assert_eq!(widget.accent(), Accent::Warning);
        widget.load = Some(0.95);
        assert_eq!(widget.accent(), Accent::Critical);
    }

    #[test]
    fn the_thresholds_move_with_the_settings() {
        // The point of the setting: on a build machine 70 % is Tuesday.
        let mut widget = SystemLoad {
            warn_at: 0.9,
            critical_at: 0.99,
            ..Default::default()
        };
        widget.load = Some(0.7);
        assert_eq!(widget.accent(), Accent::Positive);
        widget.load = Some(0.95);
        assert_eq!(widget.accent(), Accent::Warning);
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
}
