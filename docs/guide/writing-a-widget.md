# Writing a widget

A uniwidged widget is a **WASM component**. It describes what it wants shown;
the host lays it out, shapes its text, paints it, hit-tests it and reads it out
to a screen reader. You can write one in any language that targets the
component model — this guide is the Rust SDK, which is the one that exists.

```sh
uniwidged new my-widget
cd my-widget
cargo test                                    # your logic, natively
cargo build --release --target wasm32-wasip2  # the component
uniwidged install target/wasm32-wasip2/release/my_widget.wasm --allow-unsigned
uniwidged run
```

That is a working widget on your desktop. Everything below is about changing it
into the one you meant.

> **You need `rustup target add wasm32-wasip2`.** Nothing else — no C toolchain,
> no system libraries, no GPU on the build machine.

---

## 1. The one trait

```rust
use uniwidged_guest::prelude::*;

#[derive(Default)]
struct Hello;

impl Widget for Hello {
    fn view(&mut self, _cx: &Cx) -> View {
        panel(NodeId::named("root"))
            .child(text("Hello").role(Role::Title))
            .view()
    }
}

uniwidged_guest::export!(Hello);
```

Five methods, four with defaults:

| | |
|---|---|
| `view(&mut self, cx) -> View` | **Required.** What should be on screen. |
| `start(&mut self, cx) -> Result<(), String>` | Once, before the first view. Load what you remembered. |
| `on_event(&mut self, event, cx) -> bool` | React. `true` means your view would now differ. |
| `wake(&self) -> Wake` | When to be called again. Defaults to `Never`. |

`Cx` is the host. Every method on it returns a `Result`, because every one of
them is a capability a user can decline.

## 2. You describe; the host decides

There are no colours, no fonts, no pixels and no coordinates in a widget. You
say `Role::Display` and the theme decides what that is; you say
`Accent::Critical` and the theme picks a red that is legible against *this*
wallpaper at *this* contrast setting.

This is not a limitation that happens to be convenient. It is the reason a
screenful of widgets by twenty authors looks like one product instead of twenty
toolkits, and it is why a widget written today still looks right after the user
switches to dark mode, turns on high contrast, or scales their fonts.

**The vocabulary.** Containers: `panel`, `card`, `row`, `column`, `group`.
Content: `text`, `chip`, `icon`, `divider`, `spacer`, `progress`, `sparkline`,
`image`.
Modifiers chain: `.role()`, `.accent()`, `.muted()`, `.gap(n)`, `.padding(n)`,
`.align()`, `.justify()`, `.grow(n)`, `.wrap()`.

```rust
panel(NodeId::named("weather"))
    .gap(2)
    .child(text("Berlin").role(Role::Section))
    .child(
        row()
            .align(Align::Center)
            .child(text("18°").role(Role::Display))
            .child(spacer())
            .child(chip("Cloudy").accent(Accent::Neutral)),
    )
    .view()
```

`gap` and `padding` are in **theme steps**, not pixels — `.gap(2)` is two of
whatever this desktop's spacing unit is, so your layout scales with the user's
settings rather than fighting them.

**Roles**, largest to smallest: `Display`, `Title`, `Section`, `BodyStrong`,
`Body`, `Caption`. Plus the surfaces `Panel`, `Card`, `Chip` and `Plain`.

**Accents** are named by meaning, never by hue: `Neutral`, `Brand`, `Positive`,
`Warning`, `Critical`, and `Series(n)` for a categorical palette.

**A chart** is the same bargain. `sparkline(values)` takes a series of numbers
in `0.0..=1.0`, oldest first, and the host draws it — the line, the faint area
under it, a marker on the latest value, in whatever accent you gave the node.
You do not choose the stroke, the height or the colour, for the same reason you
do not choose a font: a screen of charts by different authors should be one
chart style. Give it a label, because a screen reader announces the latest
value and skips an unlabelled chart as decoration. It keeps the most recent 256
points and drops the rest, so hand it whatever history you have.

```rust
sparkline(self.history.iter().copied())
    .accent(Accent::Brand)
    .label("CPU over the last minute")
```

**A picture** is a PNG you register once and name by handle. It needs no
capability — the bytes are yours and they go on your own surface — and it is
bounded instead: 1 MiB, 1024 pixels a side, sixteen at once. Register in
`start`, because a handle belongs to the instance that made it and a restarted
widget starts again:

```rust
fn start(&mut self, cx: &Cx) -> Result<(), String> {
    self.icon = cx.register_image(include_bytes!("sun.png")).ok();
    Ok(())
}

// …and in `view`, only when there is one — a handle that failed to register
// is a gap, not an error:
row().children(self.icon.map(|icon| image(icon).label("Sunny")))
```

## 3. Ids, clicks and accessibility

Give a node an id and it becomes addressable:

```rust
chip("Refresh")
    .on_click(NodeId::named("refresh"))
    .label("Refresh the forecast")
```

`NodeId::named` hashes a string at compile time, so ids cost nothing at
runtime. Nodes without one are decoration — they are not hit-tested and do not
appear in the accessibility tree.

You are told **which node** was activated, never where the pointer was:

```rust
fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {
    match event {
        Event::Clicked(id) if *id == NodeId::named("refresh") => {
            self.refresh(cx);
            true
        }
        _ => false,
    }
}
```

`.label()` is what a screen reader says. Give one to anything whose text is not
already a sentence — an icon button with no label is a button nobody blind can
use.

**Return `true` only when the view would actually differ.** Returning `false`
when it changed leaves a stale widget on screen; returning `true` when it did
not costs one tree comparison, which is cheap. When in doubt, return `true`.

## 4. When to be called again

This is the decision that decides whether a desktop full of widgets costs a
battery.

```rust
fn wake(&self) -> Wake {
    Wake::Never                              // an external event will wake me
    Wake::at_next_multiple_of(60, now())     // the top of the next minute
    Wake::After(Duration::from_secs(30))     // a poller
    Wake::Animating                          // the refresh rate, briefly
}
```

* **`Never` is right far more often than it looks.** A widget showing data that
  arrives by push has no business owning a timer. It is the default.
* **A clock asks for an instant, not a delay.** `Wake::After(60s)` drifts a
  little further from the minute boundary on every tick, and it is simply wrong
  across a suspend, a timezone change or an NTP step. It is also why eight
  clocks cost *one* wake-up: they all name the same instant, and the scheduler
  coalesces them.
* **`Animating` is not free.** It is granted while something is genuinely
  moving. A widget that returns it across many frames without its view tree
  changing is logged and then throttled.

And do not poll. If you are waiting for something, ask for it and return
`Never`; the host wakes you when it arrives.

## 5. Capabilities

`widget.toml` is the whole of what your users are asked to approve.

```toml
name = "weather"
version = "1.0.0"
description = "Today's forecast for one place"
capabilities = ["timer", "net:api.open-meteo.com", "fs:cache"]
memory-mb = 16
```

| Capability | Grants |
|---|---|
| `timer` | returning a `Wake` other than `Never` |
| `locale` | formatting dates and times for the user's locale |
| `measure` | measuring your own text in a named role |
| `metrics` | CPU and memory counters |
| `fs:<subpath>` | reading and writing one folder inside your own directory |
| `net:<host>` | HTTPS to exactly that host — and *only* that host, redirects included |
| `link:<host>` | opening that host in the user's browser, when they click a link you drew |
| `secrets` | the OS keychain, namespaced to you |
| `command:<name>` | running one declared external command |

`net:api.open-meteo.com`, not `network` — because "this widget talks to
api.open-meteo.com" is a sentence a user can agree to, and "this widget uses
the network" is not. `fs:cache` is a grant for `cache`, not for its siblings
and not for the directory above it.

**Ask for less.** The list is shown before installation, and a widget asking for
the keychain to draw a clock is one people decline.

### Degrading well

A user can decline anything, and a widget that vanishes when they do is a worse
answer than one that says what it cannot do:

```rust
let time = cx.format_time(now).unwrap_or_else(|_| "--:--".to_string());
```

The refusals are distinguished, and the difference reaches you:

* `Denied::NotGranted` — your manifest never asked. The fix is your manifest.
* `Denied::Unavailable` — it was granted and this host cannot serve it. Nothing
  you can do; say so and carry on.
* `Denied::Malformed` — the request itself was not well-formed, like a path
  that climbs out of your directory.

### Storage

```rust
fn start(&mut self, cx: &Cx) -> Result<(), String> {
    if let Ok(Some(bytes)) = cx.read_file("state/count") {
        self.count = parse(&bytes);
    }
    Ok(())
}
```

`Ok(None)` means the file is not there, which is what your first run finds and
is **not an error**. Write with `cx.write_file(path, bytes)`; the host writes
through a temporary file and a rename, so a widget interrupted mid-write leaves
a whole file or none.

Your directory is yours. Paths are relative, `..` is refused, and so is a
symlink somebody planted that points out of it.

There are two bounds, and they refuse in different words because they have
different fixes. One file may be **4 MB** — past that the write is
`Denied::Malformed`, because it is a fact about that call and you can make it
smaller. Your whole directory may be **32 MB** — past that it is
`Denied::Unavailable`, because the request was fine and what is full is your
storage. A widget that caches should delete what it no longer needs rather than
grow towards the second one.

### Secrets

A token, a password, an API key. Not the same thing as storage, and worth the
separate capability: what goes in a file goes into a backup, a synced dotfiles
repository, and whatever is on screen when somebody shares it.

```rust
fn start(&mut self, cx: &Cx) -> Result<(), String> {
    // `Ok(None)` is your first run, exactly as with `read_file`.
    self.signed_in = matches!(cx.get_secret("api.token"), Ok(Some(_)));
    Ok(())
}
```

`cx.set_secret(key, bytes)` stores one and `cx.remove_secret(key)` forgets it —
answering `Ok(false)` if there was nothing to forget, so signing out twice is
not something to guard against. A key is at most 64 characters of
`[A-Za-z0-9._-]`, a value at most 64 KiB, and you may keep 64 keys.

Four things to do, and they are what [`widgets/vault`](https://github.com/JosunLP/uniwidged/tree/main/widgets/vault) is
for:

* **Read it in `start`, not in `view`.** `view` runs on a deadline and may run
  sixty times a second.
* **Keep the answer, not the bytes.** If all you need is "is there a token",
  store a `bool`. Every copy of a credential is somewhere it can leak from.
* **Never put it in the view.** The view is composed, hit-tested and handed to
  an accessibility tree — a secret in it is a secret a screen reader reads out.
* **Never log it.** `cx.log` goes to the host's log, which goes to a journal.

The namespace is yours and is your widget's name. There is no argument through
which to name another widget's secrets, so there is nothing here to get wrong.

What is behind the capability depends on the host, and `uniwidged doctor` says
which — a session keychain that a login unlocked, or an encrypted file that
protects a backup but not a program already running as the user. `install`
prints the same sentence at the moment somebody is asked to approve it.

### Links

`.link(url)` makes a node a link. The **host** opens it when the user clicks —
you never launch anything, and there is no function to call.

```rust
text("Forecast")
    .id(NodeId::named("forecast"))
    .link("https://example.com/forecast")
```

```toml
capabilities = ["link:example.com"]
```

`https` only. `file://`, `javascript:` and anything with a registered scheme
handler are refused whatever you were granted — none of them is a web page, and
a desktop that opened one would be running whatever that scheme is wired to.

`link:` is not `net:`, and asking for one does not get you the other. They are
different sentences to your user: "this widget talks to a server" and "this
widget can send you to a web page". Ask for whichever you actually need — a
widget that reads a local sensor and links to a forecast needs `link:` and no
network at all.

You may draw a link you were not granted. It will not open, and the desktop
will say why — you are told about the click either way, so you can show
something useful.

## 6. Settings

Some things a widget cannot pick for its user: a city, a threshold, which units.
Declare them and the host handles the rest — the form, the validation, the file.

```toml
[[setting]]
key = "warn-at"
label = "Warn above"
type = "number"
default = 0.6
min = 0.0
max = 1.0
```

```rust
fn start(&mut self, cx: &Cx) -> Result<(), String> {
    self.warn_at = match cx.setting("warn-at") {
        Some(SettingValue::Number(n)) => n as f32,
        _ => 0.6,
    };
    Ok(())
}

fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {
    if matches!(event, Event::Configured) { /* read them again */ }
    // …
}
```

`type` is `text`, `number`, `toggle` or `choice` (which takes `options`), and
every setting needs a `default` — **an unset setting is its default**, which is
what lets you read one without handling absence.

**You never parse a setting.** The host checks the type, the range and the
option list before you are started, so the `_ =>` arm above is only reachable
on a host older than your manifest. That is the whole reason settings are
declared rather than a string map: your users are told at the moment they type,
and you have no branch for what they typed.

Settings need **no capability**. They are your own manifest's, there is no
argument through which to name another widget's, and a user who declined them
would be declining you.

Read them in `start` and again on `Event::Configured` — never in `view`, which
runs on a deadline and may run sixty times a second for a value that changes
when a person changes it.

Your users run `uniwidged configure <your-widget>`.
[`widgets/system-load`](https://github.com/JosunLP/uniwidged/tree/main/widgets/system-load) is the worked example, and
`uniwidged new` scaffolds one for you.

## 7. Asking for something that takes time

A widget must never wait. Every call into your code is bounded, and a widget
that blocked on a process or a network round trip would be interrupted
mid-instruction. So anything slow works the same way, and there is one
mechanism for all of it:

```rust
fn view(&mut self, cx: &Cx) -> View {
    if self.pending.is_none() {
        // Returns immediately, with a receipt.
        match cx.run_command("uptime", &["-p"]) {
            Ok(token) => self.pending = Some(token),
            Err(denied) => self.trouble = Some(denied.to_string()),
        }
    }
    // Draw *something* now. Your widget is on screen the whole time.
    panel(NodeId::named("root"))
        .child(text(self.reading.clone().unwrap_or_else(|| "…".into())))
        .view()
}

fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {
    let Event::Delivered { token } = event else { return false };
    if self.pending != Some(*token) { return false; }
    self.pending = None;
    match cx.take_result(*token) {
        Ok(TaskResult::Command(out)) if out.succeeded() => self.reading = Some(out.text()),
        Ok(TaskResult::Command(out)) => self.trouble = Some(format!("exit {}", out.status)),
        Ok(TaskResult::Failed(why)) => self.trouble = Some(why),
        _ => {}
    }
    true
}
```

Three things worth getting right:

* **Keep the token and check it.** One counter serves the whole desktop, and
  `take_result` refuses a token that is not yours — but a widget that acted on
  any token it was handed would be a widget waiting to be confused by its own
  second request.
* **Guard the start.** A widget that calls `run_command` on every view has
  sixteen requests outstanding within a second and is then refused. That
  refusal is the host telling you about the bug; the `pending` field is the
  fix.
* **A non-zero exit is a result, not a failure.** `TaskResult::Failed` means it
  did not run — not installed, timed out. `TaskResult::Command` with a non-zero
  status means it ran and said no. Users can act on the difference.

`cx.fetch(url)` works exactly this way, and gives you a
`TaskResult::Http { status, body }`. One thing to know: a grant for
`api.example.com` is a grant for `api.example.com`, so if that server redirects
you somewhere else the request stops and you get `TaskResult::Failed` saying
where it was pointing. Declare every host you actually talk to.

### What `command:` actually grants

`command:uptime` reads as "run the `uptime` command", and it means that — but a
command can do whatever that command can do, and neither the host nor your user
can know what that is. `command:sh` is a request for arbitrary code execution
wearing a small word.

What the host does guarantee: no shell anywhere, so nothing in your arguments
can become a second command; the granted name exactly, never joined with a
path; almost no environment, because a desktop's environment holds session
tokens; no standard input; and bounds on time and output.

Ask for the smallest command that answers your question.
[`widgets/uptime`](https://github.com/JosunLP/uniwidged/tree/main/widgets/uptime) is the worked example, and it asks for
`uptime` rather than `sh` for exactly this reason.

## 8. Testing your widget

Everything except the host works natively, so `cargo test` runs your logic with
no WASM toolchain at all. Off-target, `Cx` answers `NotGranted` to everything —
which is exactly the state a user who declined puts you in, so your tests
exercise the path most widgets get wrong:

```rust
#[test]
fn a_widget_that_cannot_save_still_counts_and_says_so() {
    let mut widget = Counter::default();
    widget.on_event(&Event::Clicked(NodeId::named("increment")), &Cx::new());
    assert_eq!(widget.count, 1);
    assert!(widget.unsaved);
}
```

To see it on a desktop: `cargo build --release --target wasm32-wasip2`, then
`uniwidged install` and `uniwidged run`. `uniwidged snapshot <name>` renders it
headlessly to a PNG, which is what to put in a pull request.

**Leave the desktop running while you work on it.** `install` over a widget
that is already on the screen replaces it in place — the old instance is
stopped, its window and swapchain go with it, and the new one opens where the
old one sat with the settings it had. So the edit loop is `cargo build` and
`uniwidged install`, with nothing to restart between them. What is *not*
carried over is your widget's state: the new instance calls `start` and reads
what it saved, exactly as it would after a reboot, which is the path most worth
exercising anyway.

`uniwidged check <widget.wasm>` is the one to run before you publish, and in
CI. It parses your manifest, loads your component, checks the ABI, starts it,
draws it **with nothing granted**, and prints the consent lines your users will
see. Everything `install` would find out, without installing anything.

## 9. Publishing it

`install` refuses an unsigned widget. That is not ceremony: a `.wasm` somebody
sent you is a program, and the sandbox only starts protecting you once it is
running.

```sh
uniwidged keygen --out ~/.uniwidged-signing-key   # once, ever
uniwidged sign target/wasm32-wasip2/release/my_widget.wasm \
    --key ~/.uniwidged-signing-key                # each release
```

That writes `my_widget.wasm.sig` beside the component, and — when the manifest
it signed was your crate's `widget.toml` — a copy of it as `my_widget.toml`, the
exact bytes the signature covers. **Ship all three** side by side, and put your
public key in your README. People install it with:

```sh
uniwidged trust <your public key> --name "your name"
uniwidged install my_widget.wasm
```

**Your signature covers the manifest as well as the component.** That is
deliberate and it matters to you: without it, anybody could take your signed
widget, add `net:*.somewhere.test` to its capabilities, and pass it on with a
signature that still verifies under your name — and your users would approve it
because they recognised you. Re-sign whenever either file changes.

Keep the secret key. Anybody who has it can publish widgets as you. `keygen`
writes it `0600` and refuses to overwrite one.

For your own builds, `--allow-unsigned` says so out loud and installs anyway.

### Somewhere for people to find it

Sending people a file works, and does not scale past your friends. A registry
is one signed TOML document on a static host — no accounts, no API, nothing to
run — and once somebody has added it, your widget is `uniwidged add my-widget`:

```sh
uniwidged keygen --out ~/.uniwidged-registry-key       # once: signs your *list*
uniwidged registry publish \
    --key ~/.uniwidged-registry-key \
    --base-url https://widgets.example \
    --out index.toml \
    my_widget.wasm
# upload index.toml, index.toml.sig, and the .wasm/.toml/.sig beside them
```

The key that signs the index is **not** the key that signs your widget, and
that separation is the point: somebody who trusts your list has not thereby
trusted your code, and somebody who takes over your web host cannot sign a
widget as you. Give your version a `version` in `widget.toml` while you are
here — without one, nothing can tell your users that a newer build exists.

[Finding widgets](registries.md) is both halves in full.

## 10. What the host does to you

Running other people's code on a desktop only works because of two limits, and
you should know they are there:

* **Time.** Every call into your widget is bounded. Blow the deadline and you
  are interrupted mid-instruction — not asked to stop — and your instance ends.
  The host starts a fresh one, up to three times in a row, and then gives up on
  you. Your surface keeps its last frame throughout: a crashed widget leaves a
  stale panel, not a hole.
* **Memory.** `memory-mb` is a hard cap. Exceed it and your allocator gets
  nothing back, which reaches the host as a trap.

A fresh instance starts from `Default::default()`, which is why the counter in
§5 keeps its number on disk: **your state should be derived from your data**,
and then a restart costs nothing.

## 11. Worked examples

Seven reference widgets in this repository, one per capability (and three
for `metrics`, one per kind of answer), each small
enough to read in a sitting:

| | |
|---|---|
| [`widgets/clock`](https://github.com/JosunLP/uniwidged/tree/main/widgets/clock) | `locale`. The smallest useful widget: asks for an instant, redraws once a minute. |
| [`widgets/system-load`](https://github.com/JosunLP/uniwidged/tree/main/widgets/system-load) | `metrics` and **settings**. How often to sample, how to show a reading you could not take, and the two numbers a widget cannot pick for its user. |
| [`widgets/battery`](https://github.com/JosunLP/uniwidged/tree/main/widgets/battery) | `metrics`, when there may be **nothing to measure**. A desktop has no battery, and `Ok(None)` is "on mains power", not an empty meter — and an estimate the host does not have is not one the widget invents. |
| [`widgets/network`](https://github.com/JosunLP/uniwidged/tree/main/widgets/network) | `metrics` for a **rate**. A sparkline takes fractions and bytes per second are not one: scale against the busiest moment, with a floor, and say what the top of the chart means. |
| [`widgets/counter`](https://github.com/JosunLP/uniwidged/tree/main/widgets/counter) | `fs`. Loading in `start`, saving on change, surviving a restart. |
| [`widgets/uptime`](https://github.com/JosunLP/uniwidged/tree/main/widgets/uptime) | `command`. The asynchronous shape: ask, draw, collect on the event. |
| [`widgets/vault`](https://github.com/JosunLP/uniwidged/tree/main/widgets/vault) | `secrets`. Keeping a credential, and keeping it out of the view. |

And [`widgets/conformance`](https://github.com/JosunLP/uniwidged/tree/main/widgets/conformance), which tries every
capability and reports what happened — the fastest way to see what the sandbox
actually does.

## 12. What you cannot do

Named so nobody has to find out the hard way:

* **You cannot draw.** No canvas, no shaders, no pixels. The nearest things
  are `sparkline`, which is a series rather than a picture — you hand over
  numbers, the host draws them — and `image`, which is a picture you hand over
  whole as a PNG and the host draws at its own size. Neither lets you put a
  pixel where you want one.
* **You cannot choose a font, a colour or a size.** See §2.
* **You cannot see the pointer.** You are told which node was activated.
* **You cannot block.** There is no synchronous `fetch` and no synchronous
  `run_command`; you get a token and an event — see §7. A widget blocked on the
  network is a widget holding its deadline open.
* **You cannot invent a setting at run time.** Settings come from the manifest,
  which is what a user is shown and what a settings form is built from. A key
  you did not declare reads as `None` and there is nothing to write it with.
* **You cannot reach anything you did not declare.** WASI Preview 2 grants no
  ambient authority, so an undeclared capability is *absent* rather than
  denied. There is nothing to escape from.

---

[The API reference](../reference/api.md) is every function, type and refusal in
one page. [The architecture](../project/architecture.md) has the reasoning behind
all of this, and [the roadmap](../project/roadmap.md) says what is not built yet —
every capability in the table above is served on Linux; what is missing is
Windows and macOS.
