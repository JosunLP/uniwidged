//! A network meter: down and up, with a minute of each.
//!
//! The reference widget for `cx.network_throughput()`, and the worked example
//! for the question every chart of a *rate* runs into: **a sparkline takes
//! fractions, and bytes per second are not one.** CPU load is already
//! `0.0..=1.0`; a network is anything from silence to a gigabit. So the
//! widget scales its own history against the busiest moment in it — what
//! every network graph does — with a floor, so that a machine idling at 200
//! bytes a second draws a flat line near the bottom rather than a mountain
//! range of keep-alives.
//!
//! The scale is the widget's, and so it is shown: the caption says what the
//! top of the chart means. A chart whose axis silently rescales is one where
//! a quiet minute after a download looks exactly like a busy one.
//!
//! It reads the machine's counters and never touches the network itself, so
//! it needs `metrics` and no `net` grant — which is the right thing for a user
//! to see in the approval prompt of a network *meter*.

use uniwidged_guest::prelude::*;

/// How often to ask. Fast enough to watch a download start; the host's own
/// window is 250 ms, so asking faster than that would only get the same
/// answer back.
const PERIOD: std::time::Duration = std::time::Duration::from_secs(2);

/// A minute, at [`PERIOD`].
const HISTORY: usize = 30;

/// The least the top of the chart may mean, in bytes per second. Below this
/// a machine is idle, and drawing its keep-alives full height would show
/// noise as though it were traffic.
const FLOOR: u64 = 10_000;

#[derive(Default)]
pub struct Network {
    /// Readings, oldest first, the latest last.
    history: Vec<Throughput>,
    refused: Option<String>,
}

impl Widget for Network {
    fn view(&mut self, cx: &Cx) -> View {
        match cx.network_throughput() {
            Ok(rate) => self.record(rate),
            Err(denied) => self.refused = Some(denied.to_string()),
        }

        let latest = self.history.last().copied();
        let scale = self.scale();
        let down: Vec<f32> = self
            .history
            .iter()
            .map(|r| fraction(r.received, scale))
            .collect();
        let up: Vec<f32> = self
            .history
            .iter()
            .map(|r| fraction(r.sent, scale))
            .collect();

        panel(NodeId::named("network"))
            .gap(2)
            .child(
                row()
                    .align(Align::Center)
                    .child(text("Down").role(Role::Section))
                    .child(spacer())
                    .child(
                        chip(reading(latest.map(|r| r.received)))
                            .accent(Accent::Series(0))
                            .id(NodeId::named("down")),
                    ),
            )
            .child(
                sparkline(down)
                    .accent(Accent::Series(0))
                    .label("Received over the last minute")
                    .id(NodeId::named("down-history")),
            )
            .child(
                row()
                    .align(Align::Center)
                    .child(text("Up").role(Role::Section))
                    .child(spacer())
                    .child(
                        chip(reading(latest.map(|r| r.sent)))
                            .accent(Accent::Series(1))
                            .id(NodeId::named("up")),
                    ),
            )
            .child(
                sparkline(up)
                    .accent(Accent::Series(1))
                    .label("Sent over the last minute")
                    .id(NodeId::named("up-history")),
            )
            .child(
                // Both charts share one scale, so a slow upload under a fast
                // download looks slow, which it is.
                text(format!("top of chart: {}", rate(scale)))
                    .role(Role::Caption)
                    .muted()
                    .id(NodeId::named("scale")),
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

impl Network {
    fn record(&mut self, rate: Throughput) {
        self.history.push(rate);
        if self.history.len() > HISTORY {
            self.history.remove(0);
        }
        self.refused = None;
    }

    /// What the top of the chart means: the busiest moment in the minute,
    /// either way, and never less than [`FLOOR`].
    fn scale(&self) -> u64 {
        self.history
            .iter()
            .map(|r| r.received.max(r.sent))
            .max()
            .unwrap_or(0)
            .max(FLOOR)
    }
}

fn fraction(value: u64, scale: u64) -> f32 {
    (value as f64 / scale.max(1) as f64).clamp(0.0, 1.0) as f32
}

/// A reading for the chip, or a dash before there is one.
fn reading(value: Option<u64>) -> String {
    value.map(rate).unwrap_or_else(|| "—".to_string())
}

/// `0 B/s`, `12.3 kB/s`, `4.5 MB/s` — decimal, because a network is sold in
/// decimal units.
fn rate(bytes_per_second: u64) -> String {
    const UNITS: [&str; 4] = ["kB/s", "MB/s", "GB/s", "TB/s"];
    if bytes_per_second < 1000 {
        return format!("{bytes_per_second} B/s");
    }
    let mut value = bytes_per_second as f64 / 1000.0;
    let mut unit = 0;
    // Rounded before it is compared, so 999 990 reads "1.0 MB/s" rather than
    // "1000.0 kB/s".
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

uniwidged_guest::export!(Network);

#[cfg(test)]
mod tests {
    use super::*;

    fn at(received: u64, sent: u64) -> Throughput {
        Throughput { received, sent }
    }

    #[test]
    fn a_refused_read_shows_dashes_and_says_why() {
        let mut widget = Network::default();
        let view = format!("{:?}", widget.view(&Cx::new()));
        assert!(widget.history.is_empty());
        assert!(view.contains('—'));
        assert!(view.contains("stale"));
    }

    #[test]
    fn an_idle_machine_draws_near_the_floor_not_full_height() {
        // 200 bytes a second of keep-alives is idle. Scaled against itself it
        // would be a full-height chart of nothing.
        let mut widget = Network::default();
        widget.record(at(200, 100));
        assert_eq!(widget.scale(), FLOOR);
        assert!(fraction(200, widget.scale()) < 0.05);
    }

    #[test]
    fn the_busiest_moment_is_the_top_of_both_charts() {
        let mut widget = Network::default();
        widget.record(at(1_000_000, 50_000));
        widget.record(at(250_000, 500_000));
        assert_eq!(widget.scale(), 1_000_000);
        assert_eq!(fraction(250_000, widget.scale()), 0.25);
        assert_eq!(fraction(500_000, widget.scale()), 0.5);
    }

    #[test]
    fn the_history_is_a_minute() {
        let mut widget = Network::default();
        for i in 0..(HISTORY as u64 + 5) {
            widget.record(at(i, i));
        }
        assert_eq!(widget.history.len(), HISTORY);
        assert_eq!(widget.history[0], at(5, 5));
    }

    #[test]
    fn rates_read_the_way_a_network_is_sold() {
        assert_eq!(reading(None), "—");
        assert_eq!(rate(0), "0 B/s");
        assert_eq!(rate(999), "999 B/s");
        assert_eq!(rate(12_345), "12.3 kB/s");
        assert_eq!(rate(999_990), "1.0 MB/s");
        assert_eq!(rate(4_500_000), "4.5 MB/s");
    }

    #[test]
    fn the_caption_says_what_the_scale_is() {
        let mut widget = Network::default();
        widget.record(at(2_000_000, 0));
        let view = format!("{:?}", widget.view(&Cx::new()));
        assert!(view.contains("top of chart: 2.0 MB/s"), "{view}");
    }
}
