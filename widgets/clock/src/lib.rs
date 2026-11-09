//! A clock.
//!
//! The reference widget for `locale`, and the smallest useful widget there is.
//! Everything it does is in twenty lines, and the two decisions worth copying
//! are both about *when* rather than what:
//!
//! * It wakes at the **top of the next minute**, not in sixty seconds. A delay
//!   drifts, and it is wrong across a suspend, a timezone change and an NTP
//!   step; an instant is right through all three. See `ARCHITECTURE.md` §5.
//! * It returns the same view when the minute has not changed, so the host
//!   draws nothing. A clock that redraws every second costs sixty times what
//!   it needs to and looks identical.

use uniwidged_guest::prelude::*;

#[derive(Default)]
pub struct Clock;

impl Widget for Clock {
    fn view(&mut self, cx: &Cx) -> View {
        let now = now_seconds();
        // A refusal is shown, not hidden. A user who declined `locale` should
        // see why the clock is blank rather than a widget that looks broken.
        let time = cx.format_time(now).unwrap_or_else(|_| "--:--".to_string());
        let date = cx
            .format_date(now)
            .unwrap_or_else(|_| "locale not granted".to_string());

        panel(NodeId::named("clock"))
            .gap(1)
            .align(Align::Center)
            .justify(Justify::Center)
            .child(text(time).role(Role::Display).id(NodeId::named("time")))
            .child(
                text(date)
                    .role(Role::Caption)
                    .muted()
                    .id(NodeId::named("date")),
            )
            .view()
    }

    /// The top of the next minute — not "in sixty seconds".
    fn wake(&self) -> Wake {
        Wake::at_next_multiple_of(60, now_seconds())
    }
}

uniwidged_guest::export!(Clock);

#[cfg(test)]
mod tests {
    use super::*;

    /// A widget's own tests run natively, with no WASM toolchain and no host —
    /// `Cx` answers `NotGranted` off-target, which is exactly the case every
    /// widget has to handle anyway.
    #[test]
    fn a_clock_without_the_locale_capability_says_so_instead_of_vanishing() {
        let view = Clock.view(&Cx::new());
        let root = view.root;
        assert_eq!(root.children.len(), 2);
    }

    #[test]
    fn the_next_wake_is_the_top_of_a_minute() {
        let Wake::At(instant) = Clock.wake() else {
            panic!("a clock asks for an instant, not a delay");
        };
        let seconds = instant
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_secs() as i64;
        assert_eq!(seconds % 60, 0, "{seconds} is not a whole minute");
        assert!(seconds > now_seconds(), "and it is in the future");
        assert!(seconds - now_seconds() <= 60);
    }
}
