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
uniwidged install target/wasm32-wasip2/release/my_widget.wasm
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
Content: `text`, `chip`, `icon`, `divider`, `spacer`, `progress`, `image`.
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
| `metrics` | CPU counters |
| `fs:<subpath>` | reading and writing one folder inside your own directory |
| `net:<host>` | HTTPS to exactly that host |
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

## 6. Testing your widget

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

## 7. What the host does to you

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

## 8. Worked examples

Three reference widgets in this repository, one per capability, each small
enough to read in a sitting:

| | |
|---|---|
| [`widgets/clock`](../widgets/clock) | `locale`. The smallest useful widget: asks for an instant, redraws once a minute. |
| [`widgets/system-load`](../widgets/system-load) | `metrics`. How often to sample, and how to show a reading you could not take. |
| [`widgets/counter`](../widgets/counter) | `fs`. Loading in `start`, saving on change, surviving a restart. |

And [`widgets/conformance`](../widgets/conformance), which tries every
capability and reports what happened — the fastest way to see what the sandbox
actually does.

## 9. What you cannot do

Named so nobody has to find out the hard way:

* **You cannot draw.** No canvas, no shaders, no pixels. A `Canvas` node for
  charts is a maybe, and it would still be host-drawn from a declarative
  series.
* **You cannot choose a font, a colour or a size.** See §2.
* **You cannot see the pointer.** You are told which node was activated.
* **You cannot block.** There is no synchronous `fetch`; you get a token and an
  event. A widget blocked on the network is a widget holding its deadline open.
* **You cannot reach anything you did not declare.** WASI Preview 2 grants no
  ambient authority, so an undeclared capability is *absent* rather than
  denied. There is nothing to escape from.

---

`ARCHITECTURE.md` has the reasoning behind all of this, and `ROADMAP.md` says
what is not built yet — `net`, `secrets` and `command` currently answer
`Unavailable` however much is granted.
