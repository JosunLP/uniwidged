//! When the host is allowed to do work again.
//!
//! This is where uniwidged's performance requirement actually lives, so it is
//! worth stating the rule plainly: **there is no idle loop and there is no
//! idle frame.** Every call into a widget answers with a [`Wake`]; the
//! scheduler sleeps until the earliest one across every installed widget, and
//! between those instants the process does nothing at all.
//!
//! Ephemeris reached 0.03 s of CPU per 40 s of idle with three hand-rolled
//! timers. This is the same idea generalised to N widgets that do not know
//! about each other.
//!
//! This module only *resolves* a request into an absolute deadline. Merging
//! deadlines that are close enough to share a wake-up is the scheduler's job —
//! see `uniwidged_host::scheduler` — because it is the only participant that
//! can see all of them at once.

use std::time::{Duration, Instant, SystemTime};

/// What a widget asks for after it has been given a chance to run.
///
/// Returned from every call into guest code. The variants are ordered from
/// cheapest to most expensive, and a widget should return the cheapest one
/// that is true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Wake {
    /// Nothing scheduled. The widget will be woken by an external event — a
    /// click, a finished fetch, a configuration change — and not otherwise.
    ///
    /// This is the right answer far more often than it looks, and it is the
    /// default for a reason: a widget showing data that arrives by push has no
    /// business owning a timer.
    #[default]
    Never,
    /// A wall-clock instant. What a clock asks for, because "the top of the
    /// next minute" is a point in time and not a delay — it has to stay
    /// correct across a suspend, a timezone change and an NTP step.
    At(SystemTime),
    /// A delay from now. What a poller asks for.
    After(Duration),
    /// The display's refresh rate, for as long as something is genuinely
    /// moving.
    ///
    /// Granted for the duration of an animation the host is driving, and
    /// cleared when it ends. A widget that returns this across many frames
    /// while its view tree does not change is logged and then throttled: this
    /// variant is the one way to spend the machine's battery, and it is not
    /// available for free.
    Animating,
}

impl Wake {
    /// Resolves this request against the clock into an absolute deadline.
    ///
    /// `now` and `wall` must be read at the same moment — they are the two
    /// faces of one instant, and the conversion between an absolute wall-clock
    /// target and a monotonic deadline goes through their difference.
    /// `frame` is the display's refresh interval, used only by
    /// [`Wake::Animating`].
    pub fn deadline(self, now: Instant, wall: SystemTime, frame: Duration) -> Deadline {
        match self {
            Wake::Never => Deadline::never(),
            Wake::Animating => Deadline::at(now + frame),
            Wake::At(target) => Deadline::at(from_wall(target, now, wall)),
            Wake::After(delay) => Deadline::at(now + delay),
        }
    }
}

/// Converts an absolute wall-clock target into a monotonic deadline.
///
/// A target already in the past becomes `now`: the widget is late and should
/// run at the first opportunity, not be skipped.
fn from_wall(target: SystemTime, now: Instant, wall: SystemTime) -> Instant {
    match target.duration_since(wall) {
        Ok(remaining) => now + remaining,
        Err(_) => now,
    }
}

/// A resolved wake-up time on the monotonic clock, or none at all.
///
/// Deliberately not a bare `Option<Instant>`: "never" has to sort *after* every
/// real deadline, and `Option`'s own ordering puts `None` first. Getting that
/// backwards means one idle widget pins the scheduler to a busy loop, which is
/// the exact failure this whole module exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadline(Option<Instant>);

impl Deadline {
    /// No scheduled wake — only an external event will do it.
    pub const fn never() -> Self {
        Self(None)
    }

    pub const fn at(instant: Instant) -> Self {
        Self(Some(instant))
    }

    pub const fn instant(self) -> Option<Instant> {
        self.0
    }

    pub const fn is_never(self) -> bool {
        self.0.is_none()
    }

    /// The earlier of two deadlines, where "never" loses to everything.
    pub fn min(self, other: Self) -> Self {
        match (self.0, other.0) {
            (Some(a), Some(b)) => Self(Some(a.min(b))),
            (Some(a), None) => Self(Some(a)),
            (None, b) => Self(b),
        }
    }

    /// The earliest across a collection — what the scheduler actually sleeps
    /// until. Empty, or all-never, yields [`Deadline::never`].
    pub fn earliest(deadlines: impl IntoIterator<Item = Self>) -> Self {
        deadlines.into_iter().fold(Self::never(), Self::min)
    }

    /// How long to sleep from `now`.
    ///
    /// `None` means "block until something happens" — the host waits on its
    /// event sources with no timeout, which is what costs nothing. `Some(d)`
    /// may be zero when the deadline has already passed.
    pub fn timeout_from(self, now: Instant) -> Option<Duration> {
        self.0.map(|at| at.saturating_duration_since(now))
    }
}

impl Default for Deadline {
    fn default() -> Self {
        Self::never()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    /// A fixed pair of clock readings, so the tests do not race the real one.
    fn clocks() -> (Instant, SystemTime) {
        // An exact multiple of the grain, so quantisation is visible in the
        // arithmetic rather than hidden by whatever the wall clock happened to
        // say when the test ran.
        (
            Instant::now(),
            UNIX_EPOCH + Duration::from_secs(1_800_000_000),
        )
    }

    const FRAME: Duration = Duration::from_millis(16);

    #[test]
    fn an_idle_widget_schedules_nothing() {
        let (now, wall) = clocks();
        assert!(Wake::Never.deadline(now, wall, FRAME).is_never());
        // The scheduler must block rather than spin.
        assert_eq!(Deadline::never().timeout_from(now), None);
    }

    #[test]
    fn a_wall_clock_target_resolves_against_the_monotonic_clock() {
        // The two clocks are read at one instant and the conversion goes
        // through their difference, so an absolute target keeps its meaning
        // across a suspend or an NTP step.
        let (now, wall) = clocks();
        assert_eq!(
            Wake::At(wall + Duration::from_secs(60))
                .deadline(now, wall, FRAME)
                .instant(),
            Some(now + Duration::from_secs(60))
        );
    }

    #[test]
    fn resolution_is_exact_and_leaves_merging_to_the_scheduler() {
        // Two clocks that ask for instants 700 us apart get deadlines 700 us
        // apart. Deciding that one wake-up serves both needs every widget's
        // deadline in view at once, which only the scheduler has.
        let (now, wall) = clocks();
        let minute = wall + Duration::from_secs(60);
        let a = Wake::At(minute).deadline(now, wall, FRAME);
        let b = Wake::At(minute + Duration::from_micros(700)).deadline(now, wall, FRAME);
        assert_ne!(a, b);
        assert_eq!(
            b.instant().unwrap() - a.instant().unwrap(),
            Duration::from_micros(700)
        );
    }

    #[test]
    fn a_missed_deadline_runs_now_rather_than_being_skipped() {
        let (now, wall) = clocks();
        let overdue = Wake::At(wall - Duration::from_secs(5)).deadline(now, wall, FRAME);
        assert_eq!(overdue.instant(), Some(now));
        assert_eq!(overdue.timeout_from(now), Some(Duration::ZERO));
    }

    #[test]
    fn animation_asks_for_exactly_one_frame() {
        // Not a fixed 16 ms: the interval comes from the output the widget is
        // on, because assuming 60 Hz on a 144 Hz display is visible.
        let (now, wall) = clocks();
        assert_eq!(
            Wake::Animating.deadline(now, wall, FRAME).instant(),
            Some(now + FRAME)
        );
    }

    #[test]
    fn the_scheduler_sleeps_until_the_earliest_widget() {
        let (now, wall) = clocks();
        let deadlines = [
            Wake::Never.deadline(now, wall, FRAME),
            Wake::After(Duration::from_secs(300)).deadline(now, wall, FRAME),
            Wake::After(Duration::from_secs(60)).deadline(now, wall, FRAME),
            Wake::Never.deadline(now, wall, FRAME),
        ];
        let earliest = Deadline::earliest(deadlines);
        assert_eq!(earliest.timeout_from(now), Some(Duration::from_secs(60)));
    }

    #[test]
    fn a_desktop_of_idle_widgets_blocks_forever() {
        // Eight installed widgets, none of them scheduled. If this ever
        // returns a timeout, the host has an idle loop.
        let (now, wall) = clocks();
        let all_idle = (0..8).map(|_| Wake::Never.deadline(now, wall, FRAME));
        assert_eq!(Deadline::earliest(all_idle).timeout_from(now), None);
    }

    #[test]
    fn never_loses_to_every_real_deadline_in_either_order() {
        // `Option`'s own ordering sorts `None` first, which would make one
        // idle widget pin the scheduler to a busy loop. Hence the newtype.
        let (now, _) = clocks();
        let real = Deadline::at(now + Duration::from_secs(1));
        assert_eq!(real.min(Deadline::never()), real);
        assert_eq!(Deadline::never().min(real), real);
        assert!(Deadline::never().min(Deadline::never()).is_never());
    }
}
