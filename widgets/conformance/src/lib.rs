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
    /// The token of a command this widget started, and what came back. The
    /// asynchronous path cannot be observed in one `view` — that is the whole
    /// point of it — so it is observed across two.
    command: Option<u32>,
    command_said: Option<String>,
}

impl Widget for Conformance {
    fn start(&mut self, _cx: &Cx) -> Result<(), String> {
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
        //
        // Where a capability is *served*, the row carries what it returned
        // rather than the word "allowed": a host that answers the call and
        // answers it wrongly is not conformant either, and only the value can
        // show the difference.
        let attempts = vec![
            Attempt {
                name: "locale",
                // The epoch, so the expected answer is the same everywhere a
                // host is UTC and the difference is the host's offset rather
                // than the day this ran.
                outcome: value(cx.format_time(0)),
            },
            Attempt {
                name: "locale-date",
                outcome: value(cx.format_date(0)),
            },
            Attempt {
                name: "measure",
                outcome: measured(cx),
            },
            Attempt {
                // The number itself, rounded, so a host test can watch it
                // change when the theme does — which is the whole reason the
                // measurer follows the compositor rather than copying it.
                name: "measure-px",
                outcome: match cx.measure_text("Weather", Role::Body) {
                    Ok(width) => format!("{}", width.round() as i32),
                    Err(denied) => refusal(&denied),
                },
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
                outcome: describe_bytes(cx.read_file("cache/today.json")),
            },
            Attempt {
                name: "fs-roundtrip",
                outcome: roundtrip(cx),
            },
            Attempt {
                name: "fs-escape",
                outcome: describe_bytes(cx.read_file("../../.ssh/id_rsa")),
            },
            Attempt {
                name: "secrets",
                outcome: describe(cx.get_secret("token")),
            },
            Attempt {
                name: "metrics",
                outcome: describe(cx.cpu_load()),
            },
            Attempt {
                name: "command",
                outcome: self.command_row(cx),
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

    fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {
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
        // The other half of the asynchronous path: an answer arrives as an
        // event carrying the token that started it, and the widget collects it
        // by that token.
        if let Event::Delivered { token } = event {
            self.command_said = Some(match cx.take_result(*token) {
                Ok(TaskResult::Command(output)) => {
                    if output.succeeded() {
                        output.text()
                    } else {
                        format!("exit {}", output.status)
                    }
                }
                Ok(TaskResult::Http(_)) => "http".to_string(),
                Ok(TaskResult::Failed(_)) => "failed".to_string(),
                Err(denied) => refusal(&denied),
            });
        }
        true
    }
}

impl Conformance {
    /// Starts a command once, then reports what happened to it.
    ///
    /// Three states, and a host test can watch it move through them: the
    /// request was refused, the request is in flight, or the answer is here.
    fn command_row(&mut self, cx: &Cx) -> String {
        if let Some(said) = &self.command_said {
            return said.clone();
        }
        if self.command.is_some() {
            return "pending".to_string();
        }
        // `echo` is the one command that exists everywhere, does nothing, and
        // says what it was given — so a wrong answer is unambiguous.
        match cx.run_command("echo", &["conformance"]) {
            Ok(token) => {
                self.command = Some(token);
                "started".to_string()
            }
            Err(denied) => refusal(&denied),
        }
    }
}

/// Measures two strings and checks the answer means something.
///
/// A width is a number, and a host that returned a constant would pass any
/// test that only asked for one. So this asks for two and checks the
/// relationship: a longer string in the same role is wider, and an empty one
/// is nothing. That holds in every font, which is what makes it assertable at
/// all — the actual pixels depend on what the desktop has installed.
fn measured(cx: &Cx) -> String {
    let long = match cx.measure_text("MMMMMMMMMM", Role::Body) {
        Ok(width) => width,
        Err(denied) => return refusal(&denied),
    };
    let short = match cx.measure_text("i", Role::Body) {
        Ok(width) => width,
        Err(denied) => return refusal(&denied),
    };
    let empty = match cx.measure_text("", Role::Body) {
        Ok(width) => width,
        Err(denied) => return refusal(&denied),
    };
    if empty != 0.0 {
        return "empty-is-not-zero".to_string();
    }
    if !(long > short && short > 0.0) {
        return "degenerate".to_string();
    }
    "measured".to_string()
}

/// Writes a file and reads it back, which is the only way to find out whether
/// a granted `fs` really works.
///
/// A capability that answers without erroring has not been shown to do
/// anything. This one has: the bytes have to come back, from a directory the
/// widget cannot name and has never seen.
fn roundtrip(cx: &Cx) -> String {
    const PAYLOAD: &[u8] = b"uniwidged-roundtrip-v1";
    if let Err(denied) = cx.write_file("cache/roundtrip.bin", PAYLOAD) {
        return refusal(&denied);
    }
    match cx.read_file("cache/roundtrip.bin") {
        Ok(Some(bytes)) if bytes == PAYLOAD => "roundtrip".to_string(),
        Ok(Some(_)) => "mismatch".to_string(),
        // The host took the write and then could not find it. Worth its own
        // word: it is a broken host rather than a refused request.
        Ok(None) => "vanished".to_string(),
        Err(denied) => refusal(&denied),
    }
}

/// The outcome of one attempt, as a word the host can assert on.
///
/// `not-granted` and `unavailable` are deliberately distinguished: the first
/// means the manifest never asked, the second that it did and the host cannot
/// serve it. A widget that conflated them would tell its user the wrong thing.
fn describe<T>(result: Result<T, uniwidged_guest::Denied>) -> String {
    match result {
        Ok(_) => "allowed".to_string(),
        Err(denied) => refusal(&denied),
    }
}

/// The same, for a read: absence is not a refusal.
///
/// `absent` is what a caching widget's first run sees, and telling it apart
/// from `unavailable` is the whole reason the host answers with an option.
fn describe_bytes(result: Result<Option<Vec<u8>>, uniwidged_guest::Denied>) -> String {
    match result {
        Ok(Some(_)) => "present".to_string(),
        Ok(None) => "absent".to_string(),
        Err(denied) => refusal(&denied),
    }
}

/// A value where the host served one, and the refusal where it did not.
fn value(result: Result<String, uniwidged_guest::Denied>) -> String {
    match result {
        Ok(value) => value,
        Err(denied) => refusal(&denied),
    }
}

fn refusal(denied: &uniwidged_guest::Denied) -> String {
    use uniwidged_guest::Denied;
    match denied {
        Denied::NotGranted => "not-granted".to_string(),
        Denied::Malformed(_) => "malformed".to_string(),
        Denied::Unavailable(_) => "unavailable".to_string(),
    }
}

uniwidged_guest::export!(Conformance);
