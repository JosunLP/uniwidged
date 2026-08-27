//! A widget that counts.
//!
//! The host's scheduling, damage tracking and restart logic all need something
//! that genuinely changes over time, and the conformance widget deliberately
//! does not: it answers [`Wake::Never`] and returns the same tree every frame.
//!
//! This is the other half. It asks to be woken, changes when it is, and can be
//! told to hold still — so a test can watch a desktop go quiet and then start
//! again, which is the behaviour `ARCHITECTURE.md` §5 is about.

use uniwidged_guest::prelude::*;

#[derive(Default)]
pub struct Ticker {
    count: u32,
    /// When set, the widget keeps its schedule but stops changing. That
    /// separates two things a test needs to tell apart: a widget that is not
    /// running, and a widget that is running and has nothing new to say.
    holding: bool,
}

impl Widget for Ticker {
    fn view(&mut self, _cx: &Cx) -> View {
        panel(NodeId::named("root"))
            .gap(1)
            .child(text("Ticker").role(Role::Title))
            .child(
                text(format!("{}", self.count))
                    .role(Role::Display)
                    .id(NodeId::named("count")),
            )
            .child(
                chip(if self.holding { "held" } else { "running" })
                    .on_click(NodeId::named("hold"))
                    .label("Hold")
                    .accent(if self.holding {
                        Accent::Warning
                    } else {
                        Accent::Positive
                    }),
            )
            .view()
    }

    /// Fast on purpose: a test should not have to wait a second to see a
    /// second frame. A real widget of this kind would ask for the top of the
    /// next minute.
    fn wake(&self) -> Wake {
        Wake::After(std::time::Duration::from_millis(20))
    }

    fn on_event(&mut self, event: &Event, _cx: &Cx) -> bool {
        match event {
            Event::Clicked(id) if *id == NodeId::named("hold") => {
                self.holding = !self.holding;
                true
            }
            // Every scheduled turn arrives as a `view` call rather than an
            // event, so counting happens there — see below.
            _ => false,
        }
    }
}

impl Ticker {
    /// Advances on each view, unless held.
    ///
    /// Done in `view` rather than in a separate hook because there is no
    /// separate hook: a widget is asked what it looks like, and deciding that
    /// is when it gets to think.
    fn advance(&mut self) {
        if !self.holding {
            self.count += 1;
        }
    }
}

// The trait's `view` is what the host calls, so the counter is advanced from
// inside it. Written as a wrapper rather than inline so that `advance` above
// can say why it exists.
struct Exported(Ticker);

impl Default for Exported {
    fn default() -> Self {
        Self(Ticker::default())
    }
}

impl Widget for Exported {
    fn view(&mut self, cx: &Cx) -> View {
        self.0.advance();
        self.0.view(cx)
    }

    fn wake(&self) -> Wake {
        self.0.wake()
    }

    fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {
        self.0.on_event(event, cx)
    }
}

uniwidged_guest::export!(Exported);
