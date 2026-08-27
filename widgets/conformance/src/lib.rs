//! A widget that tries everything it is not allowed to do.
//!
//! The sandbox's tests are mostly about what it **refuses**, and a refusal can
//! only be observed from the inside. So this widget attempts every capability
//! on every `view()` and reports what happened — as its view, which the host
//! can then read and assert against.
//!
//! It is also the framework's own smallest complete example: a real component,
//! built for `wasm32-wasip2`, instantiated by the real runtime.

use uniwidged_guest::prelude::*;

/// One attempt and its outcome.
struct Attempt {
    name: &'static str,
    outcome: String,
}

#[derive(Default)]
pub struct Conformance {
    /// Set by `start`, so the host can tell a widget that ran from one that
    /// merely instantiated.
    started: bool,
    /// Counts events, so the host can check that delivery works at all.
    events: u32,
    /// Set when the host asks for it, so the deadline can be tested against a
    /// widget that really does not return.
    spin: bool,
    /// Set when the host asks for it, so the memory cap can be tested against
    /// a widget that really does try to take the machine's memory.
    grow: bool,
}

impl Widget for Conformance {
    fn start(&mut self) -> Result<(), String> {
        self.started = true;
        Ok(())
    }

    fn view(&mut self, cx: &Cx) -> View {
        if self.grow {
            // Far past any manifest's cap. The host's `ResourceLimiter`
            // refuses the growth, the guest's allocator gets nothing back,
            // and Rust aborts — which reaches the host as a trap. That whole
            // chain is what the test is for: a cap nobody has watched fail is
            // a cap nobody knows works.
            let mut hungry: Vec<Vec<u8>> = Vec::new();
            loop {
                hungry.push(vec![0u8; 8 * 1024 * 1024]);
                std::hint::black_box(&hungry);
            }
        }

        if self.spin {
            // Deliberately unbounded. The host's epoch deadline is the only
            // thing that stops this, which is exactly what the test is for.
            // `black_box` keeps the optimiser from noticing the loop does
            // nothing and removing it.
            loop {
                std::hint::black_box(0u64);
            }
        }

        // Every capability, every time. Order matters only in that it is
        // stable, so the host can address a row by index.
        let attempts = vec![
            Attempt {
                name: "locale",
                outcome: describe(cx.format_time(0)),
            },
            Attempt {
                name: "measure",
                outcome: describe(cx.measure_text("x", 0)),
            },
            Attempt {
                name: "net",
                outcome: describe(cx.fetch("https://example.com/data.json")),
            },
            Attempt {
                name: "net-malformed",
                outcome: describe(cx.fetch("file:///etc/passwd")),
            },
            Attempt {
                name: "fs-read",
                outcome: describe(cx.read_file("cache/today.json")),
            },
            Attempt {
                name: "fs-escape",
                outcome: describe(cx.read_file("../../.ssh/id_rsa")),
            },
            Attempt {
                name: "secrets",
                outcome: describe(cx.get_secret("token")),
            },
            Attempt {
                name: "metrics",
                outcome: describe(cx.cpu_load()),
            },
        ];

        // Logging is the one thing that is always available, so it is used to
        // prove the *positive* half: that the host is really there.
        cx.log("conformance ran");

        let mut root = panel(NodeId::named("root")).gap(1).child(
            text(if self.started { "started" } else { "not started" })
                .role(Role::Caption)
                .id(NodeId::named("started")),
        );
        root = root.child(
            text(format!("events: {}", self.events))
                .role(Role::Caption)
                .id(NodeId::named("events")),
        );
        for attempt in attempts {
            root = root.child(
                row()
                    .id(NodeId::named(attempt.name))
                    .child(text(attempt.name).role(Role::Caption))
                    .child(spacer())
                    .child(text(attempt.outcome).role(Role::Caption)),
            );
        }
        root.view()
    }

    fn wake(&self) -> Wake {
        Wake::Never
    }

    fn on_event(&mut self, event: &Event, _cx: &Cx) -> bool {
        self.events += 1;
        // A click on this node asks the widget to hang, so the host's deadline
        // can be tested against something that genuinely does not return.
        if let Event::Clicked(id) = event {
            if *id == NodeId::named("spin") {
                self.spin = true;
            }
            if *id == NodeId::named("grow") {
                self.grow = true;
            }
        }
        true
    }
}

/// The outcome of one attempt, as a word the host can assert on.
///
/// `not-granted` and `unavailable` are deliberately distinguished: the first
/// means the manifest never asked, the second that it did and the host cannot
/// serve it. A widget that conflated them would tell its user the wrong thing.
fn describe<T>(result: Result<T, uniwidged_guest::Denied>) -> String {
    use uniwidged_guest::Denied;
    match result {
        Ok(_) => "allowed".to_string(),
        Err(Denied::NotGranted) => "not-granted".to_string(),
        Err(Denied::Malformed(_)) => "malformed".to_string(),
        Err(Denied::Unavailable(_)) => "unavailable".to_string(),
    }
}

uniwidged_guest::export!(Conformance);
