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
    /// Counts `Configured` in particular. The variant has been in the WIT file
    /// since the first version and nothing produced it until settings existed,
    /// so "it arrived at all" is worth counting separately from "an event
    /// arrived".
    configured: u32,
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
    /// What the keychain did the first time it was tried.
    ///
    /// Once, not once per view. Two of the four calls in that sequence write
    /// the store, and a widget that rewrote a file twice on every frame would
    /// be measuring the host's disk rather than its sandbox — and would churn
    /// a real keychain, since this widget runs beside real widgets.
    secrets_said: Option<String>,
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
        let read_settings = Settings::read(cx);
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
                // `.invalid` is reserved by RFC 2606 and never resolves, so a
                // conformance run makes no meaningful network traffic — what
                // is under test is the *gate*, and the gate answers before
                // anything is sent.
                outcome: describe(cx.fetch("https://conformance.invalid/data.json")),
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
                outcome: self.secrets_row(cx),
            },
            Attempt {
                // A key the store will not take. Refused as *malformed* rather
                // than ungranted, on a host where `secrets` is granted and
                // served — which is the distinction a widget author needs and
                // the one an implementation is most likely to collapse.
                name: "secrets-key",
                outcome: describe(cx.get_secret("not a key")),
            },
            Attempt {
                // Not a capability, so it is not gated and there is no
                // refusal to report. What is reported is whether every
                // declared key came back with the declared *type* — a host
                // that handed a widget a number for a toggle would break the
                // guarantee every widget's `match` depends on.
                name: "settings",
                outcome: settings(&read_settings),
            },
            Attempt {
                name: "metrics",
                outcome: describe(cx.cpu_load()),
            },
            Attempt {
                name: "settings-values",
                outcome: setting_values(&read_settings),
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
            text(if self.started {
                "started"
            } else {
                "not started"
            })
            .role(Role::Caption)
            .id(NodeId::named("started")),
        );
        root = root.child(
            text(format!("events: {}", self.events))
                .role(Role::Caption)
                .id(NodeId::named("events")),
        );
        root = root.child(
            text(format!("configured: {}", self.configured))
                .role(Role::Caption)
                .id(NodeId::named("configured")),
        );
        // Two links, which are the one interaction the *host* acts on rather
        // than the widget. Neither needs a capability to be drawn — a widget
        // may put whatever it likes on its own surface — and what is gated is
        // the moment the browser would be launched. So these exist to be
        // clicked by a host test, not to report anything themselves.
        root = root.child(
            text("granted link")
                .role(Role::Caption)
                .id(NodeId::named("link-granted"))
                .link("https://conformance.invalid/docs"),
        );
        root = root.child(
            text("other host")
                .role(Role::Caption)
                .id(NodeId::named("link-elsewhere"))
                .link("https://elsewhere.invalid/docs"),
        );
        root = root.child(
            // Not a web page. A desktop that handed this to the system opener
            // would be letting a widget run whatever the scheme is wired to.
            text("not a web page")
                .role(Role::Caption)
                .id(NodeId::named("link-scheme"))
                .link("file:///etc/passwd"),
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
        if matches!(event, Event::Configured) {
            self.configured += 1;
        }
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
        //
        // **Only the token this row is waiting for.** This widget has two
        // requests outstanding — a command and a fetch — and acting on
        // whichever arrived first would show the network's answer in the
        // command's place. Every widget with more than one request in flight
        // has to do this, which is why the conformance widget does it too.
        if let Event::Delivered { token } = event
            && self.command == Some(*token)
        {
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
    /// The keychain lifecycle, run once and then remembered.
    fn secrets_row(&mut self, cx: &Cx) -> String {
        if let Some(said) = &self.secrets_said {
            return said.clone();
        }
        let said = secrets_roundtrip(cx);
        self.secrets_said = Some(said.clone());
        said
    }

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

/// Reads every declared setting and checks each came back as its own type.
///
/// The value is *not* asserted here — a host test sets one and reads it back
/// through the row below, which is where the values are known. What this
/// checks is the invariant the widget side depends on and cannot otherwise
/// see: that a key declared as a number arrives as a number, so a widget's
/// `match` needs no arm for anything else.
fn settings(read: &Settings) -> String {
    let wrong = |key: &str| format!("wrong-type:{key}");
    match &read.text {
        Some(SettingValue::Text(_)) => {}
        Some(_) => return wrong("some-text"),
        None => return "missing:some-text".to_string(),
    }
    match &read.number {
        Some(SettingValue::Number(_)) => {}
        Some(_) => return wrong("some-number"),
        None => return "missing:some-number".to_string(),
    }
    match &read.toggle {
        Some(SettingValue::Toggle(_)) => {}
        Some(_) => return wrong("some-toggle"),
        None => return "missing:some-toggle".to_string(),
    }
    // A choice's value is text; there is no `Choice` variant, on purpose.
    match &read.choice {
        Some(SettingValue::Text(_)) => {}
        Some(_) => return wrong("some-choice"),
        None => return "missing:some-choice".to_string(),
    }
    // A key nothing declared. `None` and not an error, and not a default:
    // there is nothing to default to.
    if read.undeclared.is_some() {
        return "invented".to_string();
    }
    "typed".to_string()
}

/// The five settings this widget looks at, read **once** per view.
///
/// Two rows report on them and each used to read them itself, which is ten
/// host calls a frame for five answers. Every one of those is charged to this
/// widget's deadline, and this widget already does more per frame than any
/// real one — so reading each key once and deriving both rows from it is the
/// same argument the rest of this project keeps making, applied to the widget
/// side of the boundary for once.
struct Settings {
    text: Option<SettingValue>,
    number: Option<SettingValue>,
    toggle: Option<SettingValue>,
    choice: Option<SettingValue>,
    undeclared: Option<SettingValue>,
}

impl Settings {
    fn read(cx: &Cx) -> Self {
        Self {
            text: cx.setting("some-text"),
            number: cx.setting("some-number"),
            toggle: cx.setting("some-toggle"),
            choice: cx.setting("some-choice"),
            undeclared: cx.setting("never-declared"),
        }
    }
}

/// The four settings' values, as one string a host test can assert on.
///
/// Separate from the row above because the two answer different questions: one
/// is about the shapes, which the widget can check by itself, and this is
/// about the values, which only the side that set them knows.
fn setting_values(read: &Settings) -> String {
    let one = |value: &Option<SettingValue>| match value {
        Some(SettingValue::Text(text)) => text.clone(),
        Some(SettingValue::Number(number)) => format!("{number}"),
        Some(SettingValue::Toggle(toggle)) => toggle.to_string(),
        None => "-".to_string(),
    };
    format!(
        "{} {} {} {}",
        one(&read.text),
        one(&read.number),
        one(&read.toggle),
        one(&read.choice)
    )
}

/// Stores a secret, reads it back, and forgets it.
///
/// The same argument as `roundtrip` above, and one more on top: this is the
/// only capability where a *stale* answer is as bad as a wrong one. So the
/// sequence is the whole lifecycle rather than a write and a read — the key is
/// absent to begin with, holds what was put in it, and is absent again after
/// being removed. A host that quietly kept the old value would pass a
/// write-then-read and fail here.
///
/// It runs on every view, so the key is set and removed again each time and
/// the keychain is left as it was found. That matters: this widget runs beside
/// real ones.
fn secrets_roundtrip(cx: &Cx) -> String {
    const KEY: &str = "conformance.token";
    const PAYLOAD: &[u8] = b"uniwidged-secret-v1";

    match cx.get_secret(KEY) {
        // The first run of a widget that has never signed in. Absence is not a
        // refusal, which is the ABI 4 → 5 change and the reason for it.
        Ok(None) => {}
        Ok(Some(_)) => return "left-over".to_string(),
        Err(denied) => return refusal(&denied),
    }
    if let Err(denied) = cx.set_secret(KEY, PAYLOAD) {
        return refusal(&denied);
    }
    match cx.get_secret(KEY) {
        Ok(Some(bytes)) if bytes == PAYLOAD => {}
        Ok(Some(_)) => return "mismatch".to_string(),
        // The host took the secret and then could not find it: a broken host
        // rather than a refused request, and worth its own word.
        Ok(None) => return "vanished".to_string(),
        Err(denied) => return refusal(&denied),
    }
    match cx.remove_secret(KEY) {
        Ok(true) => {}
        Ok(false) => return "not-removed".to_string(),
        Err(denied) => return refusal(&denied),
    }
    match cx.get_secret(KEY) {
        Ok(None) => "roundtrip".to_string(),
        Ok(Some(_)) => "still-there".to_string(),
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
