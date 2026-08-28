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

use uniwidged_guest::prelude::*;

/// How often to ask. See the module note.
const PERIOD: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Default)]
pub struct SystemLoad {
    /// The last reading, if there has been one. `None` renders as "—" rather
    /// than as 0 %, because the two mean opposite things to somebody glancing
    /// at a meter.
    load: Option<f32>,
    /// Why there is no reading, when there is not.
    refused: Option<String>,
}

impl Widget for SystemLoad {
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
    /// Colour by meaning, never by hue — the theme decides what "critical"
    /// looks like against this wallpaper at this contrast setting.
    fn accent(&self) -> Accent {
        match self.load {
            Some(load) if load >= 0.85 => Accent::Critical,
            Some(load) if load >= 0.6 => Accent::Warning,
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
}
