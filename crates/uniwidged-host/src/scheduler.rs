//! When to wake, and what to do when awake.
//!
//! The loop is three lines and the whole performance story is in them:
//!
//! ```text
//! loop {
//!     let timeout = scheduler.next_timeout(Instant::now());   // None ⇒ block
//!     shell.wait(timeout);                                    // costs nothing
//!     scheduler.run_due(Instant::now());
//! }
//! ```
//!
//! `None` is the common case on an idle desktop, and `None` means the process
//! blocks on its event sources with no timer at all. That is what Ephemeris's
//! 0.03 s per 40 s of idle actually was, and it is what this preserves across
//! N widgets that have never heard of each other.
//!
//! ## Coalescing
//!
//! Eight clock widgets all want the top of the next minute, but they compute
//! it in eight separate calls into guest code and so land microseconds apart.
//! Waking eight times where one would do costs eight scheduler passes and,
//! worse, eight chances to pull the CPU out of a sleep state.
//!
//! So the scheduler does not wait for the *earliest* deadline. It takes the
//! earliest, `D`, collects every deadline in `[D, D + SLACK]`, and waits until
//! the **last** of them. Everything in that window then runs at or after its
//! own deadline — never early, at most [`SLACK`] late.
//!
//! Waking early is not an option worth the saving: a clock woken at 11:58:59.999
//! renders the previous minute, and a widget that is occasionally wrong is worse
//! than one that is occasionally 80 ms late. This is the same trade the
//! platforms make — `SetWaitableTimerEx`'s tolerance, `dispatch_source`'s
//! leeway — and in the same direction.
//!
//! [`Wake::Animating`] deadlines take part like any other. The window is
//! shorter than a frame at every refresh rate up to 500 Hz, so an animation is
//! never delayed by more than it would have been by the frame it is waiting
//! for.

use std::time::{Duration, Instant, SystemTime};
use uniwidged_core::{Deadline, Wake, WidgetId};

/// How much later than its own deadline a widget may be run so that it can
/// share a wake-up with another.
///
/// 2 ms is chosen against what it competes with. It is long enough to catch
/// widgets whose deadlines were computed in separate calls a few hundred
/// microseconds apart — which is the case coalescing exists for — and shorter
/// than one frame at any refresh rate a desktop has, so it can never turn a
/// smooth animation into a stutter. Nothing scheduled is perceptibly late.
pub const SLACK: Duration = Duration::from_millis(2);

/// One widget's place in the schedule.
#[derive(Debug, Clone)]
struct Entry {
    id: WidgetId,
    deadline: Deadline,
    /// Suspended widgets stay in the list — they still own a surface showing
    /// their last frame — but are never scheduled.
    suspended: bool,
}

/// What one wake-up turned up.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tick {
    /// Widgets whose deadline has passed. Each gets a `view()` call; whether
    /// that produces a frame is up to whether its tree changed.
    pub due: Vec<WidgetId>,
}

impl Tick {
    /// Nothing to do. The host does no GPU work at all on such a wake — it
    /// happens when an event source fires for a widget that is not scheduled.
    pub fn is_empty(&self) -> bool {
        self.due.is_empty()
    }
}

/// Keeps every widget's next deadline and answers the only two questions the
/// loop asks.
#[derive(Debug, Default)]
pub struct Scheduler {
    entries: Vec<Entry>,
    /// The display's refresh interval, for [`Wake::Animating`].
    frame: Option<Duration>,
}

impl Scheduler {
    /// `frame` is the refresh interval of the output the widgets are on —
    /// 16.67 ms at 60 Hz, 6.94 ms at 144 Hz. Taken from the shell rather than
    /// assumed, because assuming 60 Hz on a 144 Hz display is visible.
    pub fn new(frame: Duration) -> Self {
        Self {
            entries: Vec::new(),
            frame: Some(frame),
        }
    }

    pub fn insert(&mut self, id: WidgetId) {
        if !self.entries.iter().any(|e| e.id == id) {
            self.entries.push(Entry {
                id,
                deadline: Deadline::never(),
                suspended: false,
            });
        }
    }

    pub fn remove(&mut self, id: WidgetId) {
        self.entries.retain(|e| e.id != id);
    }

    /// Records what a widget asked for after it ran.
    pub fn schedule(&mut self, id: WidgetId, wake: Wake, now: Instant, wall: SystemTime) {
        let frame = self.frame.unwrap_or(Duration::from_millis(16));
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.deadline = wake.deadline(now, wall, frame);
        }
    }

    /// Stops scheduling a widget without forgetting it. Its surface keeps the
    /// last frame it produced, which is why a crashed widget leaves a stale
    /// panel rather than a hole in the desktop.
    pub fn suspend(&mut self, id: WidgetId) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.suspended = true;
            entry.deadline = Deadline::never();
        }
    }

    /// How long to wait. `None` means block — no timer, no polling, no cost.
    ///
    /// Not simply the earliest deadline: see the module note on coalescing.
    /// The answer is the last deadline within [`SLACK`] of the earliest, so
    /// that one wake-up serves all of them and none of them is served early.
    pub fn next_timeout(&self, now: Instant) -> Option<Duration> {
        let earliest = Deadline::earliest(
            self.entries
                .iter()
                .filter(|e| !e.suspended)
                .map(|e| e.deadline),
        )
        .instant()?;
        let window_end = earliest + SLACK;
        let wake_at = self
            .entries
            .iter()
            .filter(|e| !e.suspended)
            .filter_map(|e| e.deadline.instant())
            .filter(|at| *at <= window_end)
            .max()
            .unwrap_or(earliest);
        Some(wake_at.saturating_duration_since(now))
    }

    /// Everything whose deadline has passed, cleared as it is handed out.
    ///
    /// Clearing is what stops a widget that forgets to reschedule from
    /// becoming a busy loop: it runs once, its deadline goes back to
    /// [`Deadline::never`], and it waits for an event like any idle widget.
    pub fn run_due(&mut self, now: Instant) -> Tick {
        let mut due = Vec::new();
        for entry in &mut self.entries {
            if entry.suspended {
                continue;
            }
            if entry.deadline.instant().is_some_and(|at| at <= now) {
                entry.deadline = Deadline::never();
                due.push(entry.id);
            }
        }
        Tick { due }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_millis(16);

    fn fixtures() -> (Scheduler, Instant, SystemTime) {
        (
            Scheduler::new(FRAME),
            Instant::now(),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000),
        )
    }

    #[test]
    fn a_desktop_of_idle_widgets_blocks_rather_than_polling() {
        // The single most important assertion in the project. If this ever
        // returns a timeout, uniwidged has an idle loop and every claim in
        // ARCHITECTURE.md §5 is void.
        let (mut s, now, wall) = fixtures();
        for i in 0..8 {
            s.insert(WidgetId(i));
            s.schedule(WidgetId(i), Wake::Never, now, wall);
        }
        assert_eq!(s.next_timeout(now), None);
        assert!(s.run_due(now).is_empty());
    }

    #[test]
    fn the_loop_waits_for_the_earliest_widget() {
        let (mut s, now, wall) = fixtures();
        for (i, wake) in [
            Wake::Never,
            Wake::After(Duration::from_secs(300)),
            Wake::After(Duration::from_secs(60)),
        ]
        .into_iter()
        .enumerate()
        {
            s.insert(WidgetId(i as u64));
            s.schedule(WidgetId(i as u64), wake, now, wall);
        }
        assert_eq!(s.next_timeout(now), Some(Duration::from_secs(60)));
    }

    #[test]
    fn eight_clocks_cost_one_wake_up() {
        // Coalescing, end to end. Each clock computes the top of the next
        // minute in its own call, so they differ by microseconds.
        let (mut s, now, wall) = fixtures();
        let minute = wall + Duration::from_secs(60);
        for i in 0..8u64 {
            s.insert(WidgetId(i));
            s.schedule(
                WidgetId(i),
                Wake::At(minute + Duration::from_micros(i * 37)),
                now,
                wall,
            );
        }
        let woken = now + Duration::from_secs(61);
        assert_eq!(s.run_due(woken).due.len(), 8, "all eight on one wake");
        assert_eq!(s.next_timeout(woken), None, "and none rescheduled itself");
    }

    #[test]
    fn a_widget_that_forgets_to_reschedule_does_not_spin() {
        // Running due widgets clears their deadline. Without that, a widget
        // whose `view()` forgot to return a `Wake` would be due forever and
        // the loop would never sleep again.
        let (mut s, now, wall) = fixtures();
        s.insert(WidgetId(1));
        s.schedule(WidgetId(1), Wake::After(Duration::from_secs(1)), now, wall);
        let later = now + Duration::from_secs(2);
        assert_eq!(s.run_due(later).due, vec![WidgetId(1)]);
        assert!(s.run_due(later).is_empty());
        assert_eq!(s.next_timeout(later), None);
    }

    #[test]
    fn a_suspended_widget_is_never_scheduled_again() {
        // It keeps its surface and its last frame — a crashed widget leaves a
        // stale panel, not a hole in the desktop — but it costs nothing.
        let (mut s, now, wall) = fixtures();
        s.insert(WidgetId(1));
        s.schedule(WidgetId(1), Wake::Animating, now, wall);
        s.suspend(WidgetId(1));
        assert_eq!(s.next_timeout(now), None);
        assert!(s.run_due(now + Duration::from_secs(10)).is_empty());
    }

    #[test]
    fn one_animating_widget_does_not_wake_the_idle_ones() {
        let (mut s, now, wall) = fixtures();
        s.insert(WidgetId(1));
        s.insert(WidgetId(2));
        s.schedule(WidgetId(1), Wake::Animating, now, wall);
        s.schedule(WidgetId(2), Wake::Never, now, wall);
        assert_eq!(s.next_timeout(now), Some(FRAME));
        assert_eq!(s.run_due(now + FRAME).due, vec![WidgetId(1)]);
    }

    #[test]
    fn removing_a_widget_removes_its_schedule() {
        let (mut s, now, wall) = fixtures();
        s.insert(WidgetId(1));
        s.schedule(WidgetId(1), Wake::After(Duration::from_secs(1)), now, wall);
        s.remove(WidgetId(1));
        assert_eq!(s.next_timeout(now), None);
    }
}
