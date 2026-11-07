//! A battery meter, with an hour of history.
//!
//! The reference widget for `cx.battery()`, and the worked example for the
//! one answer every other counter does not have: **there is nothing to
//! measure.** Most desktops have no battery, and `Ok(None)` is how the host
//! says so. That is not a failure, and the widget does not draw it as one —
//! it says "on mains power", in the same shape as a reading, rather than an
//! empty meter that reads as a flat battery or a warning that reads as a
//! broken host.
//!
//! It also shows how to be honest about **an estimate**. The host answers
//! `minutes_left` only where the kernel reports a rate to divide by, and a
//! widget that filled the gap with its own guess would show "9 h left" in the
//! second after a cable moved. When there is no number there is no number,
//! and the line says what the battery is doing instead.
//!
//! How often to ask: every thirty seconds. A battery changes by a percent in
//! minutes, not in seconds, and a meter that redraws more often than its
//! number can change is spending somebody's battery to watch it.

use uniwidged_guest::prelude::*;

/// How often to ask. See the module note.
const PERIOD: std::time::Duration = std::time::Duration::from_secs(30);

/// An hour of readings, at [`PERIOD`].
const HISTORY: usize = 120;

/// The fallbacks for a host that does not know this widget's manifest — the
/// only way a declared setting can come back `None`. A test asserts they are
/// the numbers `widget.toml` declares.
const LOW_AT: f32 = 0.2;
const CRITICAL_AT: f32 = 0.1;

/// What the last read said.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Reading {
    /// Nothing has been read yet.
    Waiting,
    /// The host read the machine's power supplies and found no battery.
    Mains,
    Battery(Battery),
}

pub struct BatteryWidget {
    reading: Reading,
    /// Charge readings before the current one, oldest first.
    history: Vec<f32>,
    /// Why the last read was refused, when it was.
    refused: Option<String>,
    low_at: f32,
    critical_at: f32,
}

impl Default for BatteryWidget {
    fn default() -> Self {
        Self {
            reading: Reading::Waiting,
            history: Vec::new(),
            refused: None,
            low_at: LOW_AT,
            critical_at: CRITICAL_AT,
        }
    }
}

impl Widget for BatteryWidget {
    fn start(&mut self, cx: &Cx) -> Result<(), String> {
        self.read_settings(cx);
        Ok(())
    }

    fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {
        if !matches!(event, Event::Configured) {
            return false;
        }
        let was = self.accent();
        self.read_settings(cx);
        was != self.accent()
    }

    fn view(&mut self, cx: &Cx) -> View {
        match cx.battery() {
            Ok(Some(battery)) => self.record(battery),
            Ok(None) => {
                self.reading = Reading::Mains;
                self.history.clear();
                self.refused = None;
            }
            // The last reading stays: a battery meter that blanks when the
            // host hiccups is worse than one a minute old that says so.
            Err(denied) => self.refused = Some(denied.to_string()),
        }

        let accent = self.accent();
        let mut root = panel(NodeId::named("battery")).gap(2).child(
            row()
                .align(Align::Center)
                .child(text("Battery").role(Role::Section))
                .child(spacer())
                .child(
                    chip(self.headline())
                        .accent(accent)
                        .id(NodeId::named("reading")),
                ),
        );

        if let Reading::Battery(battery) = self.reading {
            root = root
                .child(
                    progress(battery.charge)
                        .accent(accent)
                        .id(NodeId::named("meter")),
                )
                .child(
                    // The hour before now, oldest first, ending at the meter's
                    // own value so the two can never disagree about *now*.
                    sparkline(self.history.iter().copied().chain([battery.charge]))
                        .accent(accent)
                        .label("Charge over the last hour")
                        .id(NodeId::named("history")),
                );
        }

        root.child(
            text(self.status())
                .role(Role::Caption)
                .muted()
                .id(NodeId::named("status")),
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

impl BatteryWidget {
    fn record(&mut self, battery: Battery) {
        if let Reading::Battery(previous) = self.reading {
            self.history.push(previous.charge);
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
        self.reading = Reading::Battery(battery);
        self.refused = None;
    }

    /// The chip: a percentage, a dash before the first read, or a plug.
    fn headline(&self) -> String {
        match self.reading {
            Reading::Waiting => "—".to_string(),
            Reading::Mains => "Mains".to_string(),
            Reading::Battery(battery) => {
                format!(
                    "{}%",
                    (battery.charge.clamp(0.0, 1.0) * 100.0).round() as u32
                )
            }
        }
    }

    /// The line under the meter: what the battery is doing, and for how long
    /// where the host knows.
    fn status(&self) -> String {
        match self.reading {
            Reading::Waiting => "Reading the battery…".to_string(),
            Reading::Mains => "On mains power — this machine has no battery".to_string(),
            Reading::Battery(battery) => match (battery.state, battery.minutes_left) {
                (PowerState::Discharging, Some(minutes)) => {
                    format!("On battery — {} left", duration(minutes))
                }
                (PowerState::Discharging, None) => "On battery".to_string(),
                (PowerState::Charging, Some(minutes)) => {
                    format!("Charging — full in {}", duration(minutes))
                }
                (PowerState::Charging, None) => "Charging".to_string(),
                (PowerState::Idle, _) if battery.charge >= 0.995 => "Plugged in — full".to_string(),
                // Plugged in and not gaining below full is a charge limit, or
                // firmware resting the cells. Either way it is not a fault,
                // and "not charging" on its own reads like one.
                (PowerState::Idle, _) => "Plugged in — holding charge".to_string(),
            },
        }
    }

    fn read_settings(&mut self, cx: &Cx) {
        self.low_at = match cx.setting("low-at") {
            Some(SettingValue::Number(number)) => number as f32,
            _ => LOW_AT,
        };
        self.critical_at = match cx.setting("critical-at") {
            Some(SettingValue::Number(number)) => number as f32,
            _ => CRITICAL_AT,
        };
    }

    /// The colour of the meter, by meaning.
    ///
    /// A battery that is charging is never a warning, whatever its level:
    /// the thing a red meter asks somebody to do is plug it in, and it is.
    fn accent(&self) -> Accent {
        match self.reading {
            Reading::Waiting | Reading::Mains => Accent::Neutral,
            Reading::Battery(battery) if battery.state != PowerState::Discharging => {
                Accent::Positive
            }
            Reading::Battery(battery) if battery.charge <= self.critical_at => Accent::Critical,
            Reading::Battery(battery) if battery.charge <= self.low_at => Accent::Warning,
            Reading::Battery(_) => Accent::Positive,
        }
    }
}

/// `45 min`, `1 h 05 min`, `3 h`.
fn duration(minutes: u32) -> String {
    let (hours, minutes) = (minutes / 60, minutes % 60);
    match (hours, minutes) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes:02} min"),
    }
}

uniwidged_guest::export!(BatteryWidget);

#[cfg(test)]
mod tests {
    use super::*;

    fn on_battery(charge: f32, minutes_left: Option<u32>) -> Battery {
        Battery {
            charge,
            state: PowerState::Discharging,
            minutes_left,
        }
    }

    #[test]
    fn a_refused_read_shows_a_dash_and_says_why() {
        // Off-target `Cx` refuses everything, which is what a user who
        // declined `metrics` puts the widget in.
        let mut widget = BatteryWidget::default();
        let view = format!("{:?}", widget.view(&Cx::new()));
        assert_eq!(widget.reading, Reading::Waiting);
        assert!(widget.refused.is_some());
        assert!(view.contains('—'));
        assert!(view.contains("stale"));
    }

    #[test]
    fn no_battery_is_mains_power_not_an_empty_one() {
        let widget = BatteryWidget {
            reading: Reading::Mains,
            ..Default::default()
        };
        assert_eq!(widget.headline(), "Mains");
        assert!(widget.status().contains("mains"));
        // Neutral: there is nothing to warn about on a desktop.
        assert_eq!(widget.accent(), Accent::Neutral);
    }

    #[test]
    fn the_time_left_is_shown_only_where_the_host_knows_it() {
        let mut widget = BatteryWidget::default();
        widget.record(on_battery(0.5, Some(185)));
        assert_eq!(widget.status(), "On battery — 3 h 05 min left");
        widget.record(on_battery(0.5, None));
        assert_eq!(widget.status(), "On battery");
        widget.record(Battery {
            charge: 0.8,
            state: PowerState::Charging,
            minutes_left: Some(25),
        });
        assert_eq!(widget.status(), "Charging — full in 25 min");
    }

    #[test]
    fn plugged_in_and_not_gaining_is_not_a_fault() {
        let mut widget = BatteryWidget::default();
        widget.record(Battery {
            charge: 1.0,
            state: PowerState::Idle,
            minutes_left: None,
        });
        assert_eq!(widget.status(), "Plugged in — full");
        widget.record(Battery {
            charge: 0.8,
            state: PowerState::Idle,
            minutes_left: None,
        });
        assert_eq!(widget.status(), "Plugged in — holding charge");
        assert_eq!(widget.accent(), Accent::Positive);
    }

    #[test]
    fn the_meter_warns_only_on_battery() {
        let mut widget = BatteryWidget::default();
        widget.record(on_battery(0.5, None));
        assert_eq!(widget.accent(), Accent::Positive);
        widget.record(on_battery(0.15, None));
        assert_eq!(widget.accent(), Accent::Warning);
        widget.record(on_battery(0.05, None));
        assert_eq!(widget.accent(), Accent::Critical);
        // Five percent and charging: the thing red would ask for is done.
        widget.record(Battery {
            charge: 0.05,
            state: PowerState::Charging,
            minutes_left: None,
        });
        assert_eq!(widget.accent(), Accent::Positive);
    }

    #[test]
    fn the_thresholds_move_with_the_settings() {
        let mut widget = BatteryWidget {
            low_at: 0.4,
            critical_at: 0.3,
            ..Default::default()
        };
        widget.record(on_battery(0.35, None));
        assert_eq!(widget.accent(), Accent::Warning);
    }

    #[test]
    fn the_fallbacks_are_the_numbers_the_manifest_declares() {
        let manifest = include_str!("../widget.toml");
        assert!(manifest.contains(&format!("default = {LOW_AT}")));
        assert!(manifest.contains(&format!("default = {CRITICAL_AT}")));
    }

    #[test]
    fn a_host_that_knows_no_settings_gets_the_declared_defaults() {
        let mut widget = BatteryWidget {
            low_at: 0.0,
            critical_at: 0.0,
            ..Default::default()
        };
        widget.read_settings(&Cx::new());
        assert_eq!(widget.low_at, LOW_AT);
        assert_eq!(widget.critical_at, CRITICAL_AT);
    }

    #[test]
    fn the_history_is_an_hour_and_ends_before_now() {
        let mut widget = BatteryWidget::default();
        for i in 0..(HISTORY + 10) {
            widget.record(on_battery(1.0 - i as f32 / 1000.0, None));
        }
        assert_eq!(widget.history.len(), HISTORY);
        let Reading::Battery(now) = widget.reading else {
            panic!("a reading");
        };
        assert!(widget.history.last().copied().unwrap() > now.charge);
    }

    #[test]
    fn going_onto_mains_forgets_the_history() {
        // A battery that disappeared — a laptop docked with the battery
        // removed — has no hour to show.
        let mut widget = BatteryWidget::default();
        widget.record(on_battery(0.5, None));
        widget.record(on_battery(0.4, None));
        widget.reading = Reading::Mains;
        widget.history.clear();
        let view = format!("{:?}", widget.view(&Cx::new()));
        assert!(!view.contains("Sparkline"), "{view}");
    }

    #[test]
    fn durations_read_like_a_person_wrote_them() {
        assert_eq!(duration(0), "0 min");
        assert_eq!(duration(45), "45 min");
        assert_eq!(duration(60), "1 h");
        assert_eq!(duration(65), "1 h 05 min");
        assert_eq!(duration(600), "10 h");
    }

    #[test]
    fn only_a_configured_event_re_reads() {
        let mut widget = BatteryWidget::default();
        assert!(!widget.on_event(&Event::EnvironmentChanged, &Cx::new()));
        assert!(!widget.on_event(&Event::Configured, &Cx::new()));
    }
}
