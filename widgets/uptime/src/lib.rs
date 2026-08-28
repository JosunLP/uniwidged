//! How long this machine has been up.
//!
//! The reference widget for `command`, and the one that shows what an
//! *asynchronous* capability looks like from inside a widget. The pattern is
//! the same for `net` when it lands, because there is one mechanism:
//!
//! 1. Ask. You get a **token** back immediately — the host never lets a widget
//!    wait, because a widget that waits is a widget holding its deadline open.
//! 2. Return a view that says you are waiting. This is not a nicety: your
//!    widget is on somebody's screen the whole time.
//! 3. Collect on the `Delivered` event, by the token you kept.
//!
//! ## On asking for `command:`
//!
//! `uptime` reads one number out of the kernel and prints it. That is why it is
//! the example rather than something more useful: a command can do whatever
//! that command can do, and there is no way for the host to know what that is.
//! Ask for the smallest one that answers your question.

use uniwidged_guest::prelude::*;

/// How often to ask again. Uptime changes by a minute a minute, so a minute is
/// as often as there is anything new to show — and the host coalesces widgets
/// that name the same instant, so a screenful of these costs one wake-up.
fn next_minute() -> Wake {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0);
    Wake::at_next_multiple_of(60, now)
}

#[derive(Default)]
pub struct Uptime {
    /// The token of a request in flight, if there is one. Also what stops the
    /// widget starting a second request every time it is drawn.
    pending: Option<u32>,
    /// The last answer. Kept across a refresh so the widget shows the previous
    /// reading rather than blanking for as long as the command takes.
    reading: Option<String>,
    /// Why there is no reading, when there is not.
    trouble: Option<String>,
}

impl Widget for Uptime {
    fn view(&mut self, cx: &Cx) -> View {
        // Start one if nothing is in flight. A widget that started one on every
        // view would have sixteen outstanding within a second and be refused —
        // which is the host telling its author about this exact bug.
        if self.pending.is_none() {
            match cx.run_command("uptime", &["-p"]) {
                Ok(token) => self.pending = Some(token),
                Err(denied) => self.trouble = Some(denied.to_string()),
            }
        }

        let body = match (&self.reading, &self.trouble) {
            (Some(reading), _) => text(reading.clone()).role(Role::Body),
            (None, Some(_)) => text("unavailable").role(Role::Body).muted(),
            // The first second, before the first answer. Saying so beats an
            // empty panel that looks broken.
            (None, None) => text("…").role(Role::Body).muted(),
        };

        panel(NodeId::named("uptime"))
            .gap(1)
            .child(text("Uptime").role(Role::Section))
            .child(body.id(NodeId::named("reading")))
            .children(self.trouble.as_ref().map(|why| {
                text(why.clone())
                    .role(Role::Caption)
                    .accent(Accent::Warning)
                    .id(NodeId::named("trouble"))
            }))
            .view()
    }

    fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {
        let Event::Delivered { token } = event else {
            return false;
        };
        // Only ours, and only the one we are waiting for. A token that is not
        // the pending one is somebody else's problem — the host does not
        // deliver those, and checking is cheaper than trusting.
        if self.pending != Some(*token) {
            return false;
        }
        self.pending = None;

        match cx.take_result(*token) {
            Ok(TaskResult::Command(output)) if output.succeeded() => {
                self.reading = Some(output.text());
                self.trouble = None;
            }
            // It ran and said no. Different from not being able to run it, and
            // shown differently.
            Ok(TaskResult::Command(output)) => {
                self.trouble = Some(format!("uptime exited {}", output.status));
            }
            Ok(TaskResult::Failed(why)) => self.trouble = Some(why),
            Ok(TaskResult::Http(_)) => self.trouble = Some("unexpected answer".into()),
            Err(denied) => self.trouble = Some(denied.to_string()),
        }
        true
    }

    fn wake(&self) -> Wake {
        next_minute()
    }
}

uniwidged_guest::export!(Uptime);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_widget_that_cannot_run_the_command_says_so_and_still_draws() {
        // Off-target `Cx` refuses everything, which is the state a user who
        // declined `command:uptime` puts this in.
        let mut widget = Uptime::default();
        let view = widget.view(&Cx::new());
        assert!(widget.pending.is_none());
        assert!(widget.trouble.is_some());
        assert!(format!("{view:?}").contains("unavailable"));
    }

    #[test]
    fn an_answer_for_a_token_we_are_not_waiting_on_is_ignored() {
        // The host does not deliver somebody else's token, and a widget that
        // acted on one anyway would show whatever arrived.
        let mut widget = Uptime::default();
        widget.pending = Some(7);
        assert!(!widget.on_event(&Event::Delivered { token: 9 }, &Cx::new()));
        assert_eq!(widget.pending, Some(7), "still waiting for its own");
    }

    #[test]
    fn the_next_wake_is_a_whole_minute() {
        let Wake::At(_) = next_minute() else {
            panic!("uptime asks for an instant, so a screenful of widgets share one wake-up");
        };
    }
}
