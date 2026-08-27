//! The two limits that hold even when nothing is granted.
//!
//! A widget with no capabilities at all can still allocate until the machine
//! swaps, or loop forever inside `view()`. Neither is a capability question, so
//! neither is solved by the capability system.

use std::time::Duration;

/// Per-instance resource limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Linear-memory ceiling. Declared in the manifest and shown to the user,
    /// because a widget asking for 512 MB is telling you something.
    pub memory_bytes: usize,
    /// How long one call into guest code may take before it is interrupted.
    ///
    /// Enforced with Wasmtime epoch interruption, which is a check on a
    /// counter rather than a signal, so it works on a loop with no allocation
    /// and no call in it.
    pub call_deadline: Duration,
    /// How many consecutive deadline overruns before the widget is suspended
    /// rather than merely interrupted.
    ///
    /// One overrun is a slow machine under load. Three in a row is a bug, and
    /// re-running a bug sixty times a second is how a widget framework becomes
    /// the reason a laptop is warm.
    pub overruns_before_suspend: u8,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // Enough for a widget holding a few hundred calendar entries and
            // its own scratch, and far below the point where eight of them
            // matter on an 8 GB machine.
            memory_bytes: 32 * 1024 * 1024,
            // A view rebuild is string formatting and tree building. 16 ms is
            // one frame at 60 Hz, and a widget that needs more than a whole
            // frame to describe itself is doing work that belongs off the wake
            // path.
            call_deadline: Duration::from_millis(16),
            overruns_before_suspend: 3,
        }
    }
}

/// What running a widget's call produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    /// Interrupted at the deadline. The surface keeps its last frame.
    TimedOut,
    /// Suspended after repeated overruns, or after a trap. Shown as errored;
    /// restarted only when the user asks or the widget is reinstalled.
    Suspended,
}

/// Tracks consecutive overruns for one widget instance.
#[derive(Debug, Clone, Copy, Default)]
pub struct Health {
    consecutive_overruns: u8,
}

impl Health {
    /// Records a completed call. Resets the count — a widget that recovers is
    /// not held against its worst minute.
    pub fn completed(&mut self) -> Outcome {
        self.consecutive_overruns = 0;
        Outcome::Completed
    }

    /// Records an overrun and decides whether the widget survives it.
    pub fn timed_out(&mut self, limits: &Limits) -> Outcome {
        self.consecutive_overruns = self.consecutive_overruns.saturating_add(1);
        if self.consecutive_overruns >= limits.overruns_before_suspend {
            Outcome::Suspended
        } else {
            Outcome::TimedOut
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_slow_call_is_forgiven_and_a_pattern_is_not() {
        let limits = Limits::default();
        let mut health = Health::default();
        assert_eq!(health.timed_out(&limits), Outcome::TimedOut);
        assert_eq!(health.timed_out(&limits), Outcome::TimedOut);
        assert_eq!(health.timed_out(&limits), Outcome::Suspended);
    }

    #[test]
    fn a_widget_that_recovers_is_not_held_to_its_worst_minute() {
        // A machine under load makes every widget miss a deadline. Counting
        // those cumulatively would suspend well-behaved widgets after a
        // compile.
        let limits = Limits::default();
        let mut health = Health::default();
        health.timed_out(&limits);
        health.timed_out(&limits);
        assert_eq!(health.completed(), Outcome::Completed);
        assert_eq!(health.timed_out(&limits), Outcome::TimedOut);
    }
}
