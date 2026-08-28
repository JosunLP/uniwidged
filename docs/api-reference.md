# API reference

Everything a widget can call, name or be told, in one page. It is the companion
to [`writing-a-widget.md`](writing-a-widget.md): that one is the argument and
the worked examples, this one is the surface.

The authority is `wit/uniwidged.wit` — the interface itself, vendored so that a
widget built last month keeps working. Everything below is what the Rust SDK
makes of it. A widget written in another language that targets the component
model implements the same world and gets the same guarantees; the names change
and nothing else does.

**ABI 5.** Checked at instantiation, before anything else runs — see
[ABI history](#abi-history).

---

## Contents

* [The trait](#the-trait) · [`export!`](#export)
* [Building a view](#building-a-view) — [containers](#containers),
  [content](#content), [modifiers](#modifiers)
* [Style](#style) — [`Role`](#role), [`Accent`](#accent),
  [layout](#layout-modifiers)
* [Identity and interaction](#identity-and-interaction)
* [Scheduling](#scheduling)
* [Events](#events)
* [Host functions](#host-functions) — one section per capability
* [Refusals](#refusals)
* [The manifest](#the-manifest)
* [Limits](#limits)
* [ABI history](#abi-history)

---

## The trait

```rust
pub trait Widget: Default {
    fn view(&mut self, cx: &Cx) -> View;
    fn wake(&self) -> Wake { Wake::Never }
    fn start(&mut self, cx: &Cx) -> Result<(), String> { Ok(()) }
    fn on_event(&mut self, event: &Event, cx: &Cx) -> bool { false }
}
```

| | |
|---|---|
| `start` | Called once, before the first view. Where a widget loads what it saved. An `Err` stops the widget from starting at all — right for a configuration it cannot work without, wrong for a missing cache. |
| `view` | Called whenever the host thinks something may have changed. Build the tree fresh; returning an equal tree draws nothing, and the comparison costs less than the caching would. |
| `wake` | When to be called again. See [scheduling](#scheduling). |
| `on_event` | React. Return `true` if the view would now differ. `false` when it would have is a stale widget; `true` when it would not costs one tree comparison. When in doubt, `true`. |

Every one of these runs on a **call deadline** — 16 ms by default. Past it the
instance is interrupted and the widget is restarted. There is no way to ask for
longer, and nothing in the host functions below blocks.

### `export!`

```rust
uniwidged_guest::export!(MyWidget);
```

Exactly one per component. Off `wasm32` it expands to a type check and nothing
else, so a widget crate still compiles and still runs its own unit tests with
no WASM toolchain installed.

---

## Building a view

`use uniwidged_guest::prelude::*;` brings in all of it. Every builder returns a
`Node`; `.view()` on the root turns it into the `View` that `view` returns.

```rust
panel(NodeId::named("weather"))
    .gap(2)
    .child(text("Berlin").role(Role::Section))
    .child(text("18°").role(Role::Display).id(NodeId::named("temperature")))
    .child(
        row()
            .gap(1)
            .child(chip("Refresh").on_click(NodeId::named("refresh")))
            .child(spacer()),
    )
    .view()
```

### Containers

| | |
|---|---|
| `panel(id)` | The root of a widget: a rounded, translucent surface. Takes an id because a widget's own root is addressable. |
| `card()` | A panel inside a panel, for grouping. |
| `group()` | No appearance of its own; a box to lay children out in. |
| `row()` / `column()` | `group()` with the direction already set. |

### Content

| | |
|---|---|
| `text(s)` | A string. Shaped, wrapped and truncated by the host. |
| `chip(s)` | Text in a pill — what a small button is made of. |
| `icon(Icon)` | One of `Refresh`, `Settings`, `Close`, `Chevron`, `Check`, `Alert`, `Offline`. The set is closed: a widget cannot ship a glyph. |
| `image(ImageHandle)` | An image the host holds. |
| `progress(f32)` | A bar, `0.0` to `1.0`, clamped. |
| `divider()` | A rule. |
| `spacer()` | Takes the space nothing else wanted. What pushes a row's contents apart. |

### Modifiers

Chained onto any node.

| | |
|---|---|
| `.id(NodeId)` | Names the node. Required for anything clickable or focusable. |
| `.child(Node)` / `.children(iter)` | Adds children. `children` takes anything iterable, including an `Option`, which is how a conditional row is written without a branch. |
| `.role(Role)` / `.accent(Accent)` / `.muted()` | [Style](#style). |
| `.row()` / `.column()` / `.gap(n)` / `.padding(n)` / `.align(_)` / `.justify(_)` / `.grow(n)` / `.wrap()` | [Layout](#layout-modifiers). |
| `.on_click(NodeId)` | Makes the node a button, and names what a click reports. |
| `.link(url)` | Marks it a link, which is what an assistive technology is told. **Nothing opens it yet** — see `ROADMAP.md`; handing a widget a way to make the host launch an arbitrary URL is a capability, not a modifier, and it has not been designed. |
| `.label(s)` | The accessible name, when the visible text is not one. |
| `.tooltip(s)` | Hover text. |
| `.disabled(bool)` / `.selected(bool)` / `.checked(bool)` | State, for both the appearance and the accessibility tree. |

---

## Style

A widget names style; the theme decides what it is. This is what makes a screen
full of third-party widgets look like one product, and it is not negotiable:
there is no way to ask for a colour, a font, a weight or a size.

### `Role`

| | |
|---|---|
| `Plain` · `Panel` · `Card` · `Chip` | Surfaces. |
| `Display` | The one number a widget exists to show. |
| `Title` · `Section` | Headings. |
| `Body` · `BodyStrong` | Running text. |
| `Caption` | Secondary text. |

Numeric roles get tabular figures, so a clock does not change width as the
minute turns.

### `Accent`

Named by meaning, never by hue: `Neutral`, `Brand`, `Positive`, `Warning`,
`Critical`, and `Series(u8)` for the *n*th line of a chart. `.muted()` lowers a
node's emphasis within whatever accent it has — except at high contrast, where
opacity is not available and weight carries the hierarchy instead.

### Layout modifiers

`gap` and `padding` are in **theme steps, not pixels**, so spacing scales with
the desktop rather than with a number somebody typed. `align` is
`Start`/`Center`/`End`/`Stretch` across the direction; `justify` is
`Start`/`Center`/`End`/`SpaceBetween` along it. `grow(n)` takes a share of what
is left over. `wrap()` allows a row to become several.

---

## Identity and interaction

```rust
NodeId::named("refresh")   // FNV-1a of the string; stable across builds
NodeId(7)                  // or count them yourself
NodeId::ANONYMOUS          // decoration; nothing may address it
```

The host lays out, so the host hit-tests: a widget is told **which node** was
activated and never where the pointer was. There is no pointer position in this
API at all, and no way to derive one.

---

## Scheduling

```rust
fn wake(&self) -> Wake
```

| | |
|---|---|
| `Wake::Never` | Nothing scheduled. The default, and right more often than it looks: a widget whose data arrives by event has no business owning a timer. |
| `Wake::At(SystemTime)` | A wall-clock instant. What a clock asks for — "the top of the next minute" stays correct across a suspend, a timezone change and an NTP step, and "in 60 seconds" does not. |
| `Wake::After(Duration)` | A delay. What a poller asks for. |
| `Wake::Animating` | The display's refresh rate, while something is genuinely moving. A widget that returns this while its view does not change is logged and then throttled. |

Anything other than `Never` needs the `timer` capability.

---

## Events

```rust
pub enum Event {
    Clicked(NodeId),
    Toggled(NodeId, bool),
    Delivered { token: u32 },
    Configured,
    EnvironmentChanged,
}
```

`Delivered` carries the token an asynchronous request returned. A widget with
more than one request outstanding **must** check the token — acting on whichever
arrived first is how the network's answer ends up in the command's place.

`EnvironmentChanged` means the appearance, contrast or locale changed. The view
is rebuilt either way; a widget rarely needs to do anything.

---

## Host functions

All on `Cx`, all gated by a capability the manifest declared and the user
approved. An ungranted call **returns** `Denied` rather than trapping, so a
widget can degrade.

### Always available

```rust
cx.log(message: &str)
```

Goes to the host's log, prefixed with the widget's name. Never a place for a
secret. Bounded: 256 lines per call, 512 bytes per line.

### `timer`

No function. It gates returning a [`Wake`](#scheduling) other than `Never`.

### `locale`

```rust
cx.format_time(epoch_seconds: i64) -> Result<String, Denied>
cx.format_date(epoch_seconds: i64) -> Result<String, Denied>
```

The machine's own UTC offset, daylight saving included, with the hour cycle and
field order from `LC_TIME`. Numerals only — deliberately not CLDR, and there
are no month or weekday names.

### `measure`

```rust
cx.measure_text(text: &str, role: Role) -> Result<f32, Denied>
```

Logical pixels, as the layout will actually measure it: same font, same size,
same weight, same digit spacing, in the theme the widget is about to be drawn
in. For a widget making its own truncation decisions — anything else is
deciding against a guess.

### `metrics`

```rust
cx.cpu_load() -> Result<f32, Denied>
```

`0.0` to `1.0` across all cores. Never blocks: the first call answers with the
average since boot, and each one after it with the window since the last
sample. Two calls inside 250 ms get the same answer rather than noise.

### `fs:<subpath>`

```rust
cx.read_file(path: &str) -> Result<Option<Vec<u8>>, Denied>
cx.write_file(path: &str, bytes: &[u8]) -> Result<(), Denied>
```

Inside the widget's own directory, under the granted subpath. `Ok(None)` is a
file that is not there — the first run of a caching widget, and not an error.
Writes go through a temporary file and a rename, so an interrupted widget leaves
a whole file or none.

Paths are relative and stay relative. `..` is refused, and so is a symlink
somebody planted that resolves outside — checked textually *and* again after
the filesystem has resolved it, because only the second check can see the
symlink.

**Bounds.** 4 MB per file (`Malformed` — the call is fixable) and 32 MB per
widget altogether (`Unavailable` — the call was fine and the storage is full).

### `secrets`

```rust
cx.get_secret(key: &str) -> Result<Option<Vec<u8>>, Denied>
cx.set_secret(key: &str, value: &[u8]) -> Result<(), Denied>
cx.remove_secret(key: &str) -> Result<bool, Denied>
```

The keychain, namespaced to this widget — there is no argument through which to
name another's. `Ok(None)` is a key that was never set, which is the first run.
`remove_secret` answers `Ok(false)` when there was nothing to remove, so signing
out twice is not something to guard against.

A key is 1–64 characters of `[A-Za-z0-9._-]`; a value is at most 64 KiB; a
widget may keep 64 keys. All three are `Malformed`.

Read in `start`, keep the answer rather than the bytes, and never put a secret
in the view or the log — the view is handed to an accessibility tree.

What is behind the capability depends on the host. `uniwidged doctor` says
which, and `install` prints the same sentence when somebody is asked to approve
it.

### `net:<host>`

```rust
cx.fetch(url: &str) -> Result<u32, Denied>   // a token
```

HTTPS only, to exactly the granted host. Returns immediately; the answer arrives
as `Event::Delivered` and is collected with `take_result`.

**A grant for a host is a grant for that host.** Redirects are followed by the
host and every hop goes back through the same policy, so a `302` to somewhere
else is refused and says where it was being sent. A downgrade to plain HTTP is
refused the same way. There are no cookies, no credentials, no headers and no
method — the interface is a URL.

### `command:<name>`

```rust
cx.run_command(name: &str, args: &[&str]) -> Result<u32, Denied>   // a token
```

The granted name exactly, looked up on `PATH` and run **without a shell** — so
nothing in `args` can become a second command. Almost no environment (`PATH`
and nothing else, because a desktop's holds session tokens), no standard input,
and bounds on time and output enforced by killing.

At most 64 arguments totalling 16 KiB.

A command can do whatever that command can do. `command:sh` is arbitrary code
execution wearing a small word.

### Collecting an answer

```rust
cx.take_result(token: u32) -> Result<TaskResult, Denied>

pub enum TaskResult {
    Command(CommandOutput),   // .status, .stdout, .stderr, .text(), .succeeded()
    Http(HttpResponse),       // .status, .body
    Failed(String),           // the work did not happen
}
```

One mechanism for every kind of slow thing. A token may be taken once, and only
by the widget that started it; a token that is not yours reads exactly like one
that never existed. Asking before the `Delivered` event answers `Unavailable`.

`Failed` is distinct from a non-zero exit status or a `404`, both of which *are*
answers.

Sixteen requests outstanding per widget. A widget that starts one on every view
is refused rather than queued, because a queue hides the bug.

---

## Refusals

```rust
pub enum Denied {
    NotGranted,
    Unavailable(String),
    Malformed(String),
}
```

Three different facts, and a widget that collapses them tells somebody the
wrong thing:

| | Means | Fixed by |
|---|---|---|
| `NotGranted` | The manifest never asked, or the user declined. | The manifest, or the user. |
| `Unavailable` | Granted, and this host cannot serve it. | Nobody. Say so and carry on. |
| `Malformed` | The request itself was not well-formed. | The widget's author. |

---

## The manifest

`widget.toml`, beside the component and covered by the same signature.

```toml
name = "weather"
version = "1.0.0"
description = "Today's forecast for one place"
capabilities = ["timer", "net:api.open-meteo.com", "fs:cache"]
memory-mb = 16
```

`name` becomes a directory and a keychain namespace, so it is strict: 1–64
characters, lower-case ASCII, digits and `-`, starting with a letter and not
ending with `-`.

| Capability | Written |
|---|---|
| `timer` · `locale` · `measure` · `metrics` · `secrets` | bare |
| `fs:<subpath>` | a relative path with no `..` |
| `net:<host>` | one host, or `*.example.com` for its subdomains |
| `command:<name>` | a program name with no path separator |

The signature covers the component **and** the manifest together. A signature
over the component alone would let anybody widen `capabilities` and pass it on
with a signature that still verified.

---

## Limits

| | |
|---|---|
| Call deadline | 16 ms, then the instance is interrupted |
| Memory | `memory-mb`, enforced by the runtime's resource limiter |
| Restarts | three failures, then the widget is suspended |
| View tree | bounded node count, bounded string length, no cycles — indices must increase, which makes a cycle inexpressible rather than merely detected |
| Storage | 4 MB per file, 32 MB per widget |
| Secrets | 64 keys per widget, 64 KiB each, 64-character key names |
| Tasks | 16 outstanding per widget, 8 running per desktop |
| Command | 64 arguments, 16 KiB total |
| Log | 256 lines per call, 512 bytes per line |

---

## ABI history

`abi-version` is checked at instantiation, before `start`, and a mismatch is
refused in a sentence naming both numbers.

| | |
|---|---|
| **1 → 2** | `read-file` answers `ok(none)` for a file that is not there. A caching widget's first run finds nothing, and that is its normal case rather than its failure case; the previous shape made it read an error string to find out. |
| **2 → 3** | `measure-text` takes the `role` enum rather than a `u8` index into it. The index had no meaning an author could look up and nothing stopped one passing 200. |
| **3 → 4** | One asynchronous mechanism instead of one per capability. `take-response` is gone; `take-result` collects any request by token and says what kind of answer it is. `run-command` joins `fetch` as the second thing that can start one. |
| **4 → 5** | `get-secret` answers `ok(none)` for a key that was never set — the 1 → 2 change made again, in the other place where absence was being reported as an error. `remove-secret` is new: a widget that cannot forget a credential is one that keeps it forever. |
