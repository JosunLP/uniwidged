# Core concepts

Five ideas explain almost every decision in uniwidged. Once they click, the API
reads as the only reasonable shape it could have.

## The host owns the pixels

A widget never draws. It returns a **view tree** — a description of what it
wants shown — and the host does the rest:

```text
 widget.wasm                         host
 ───────────                         ────────────────────────────────────────
 view(cx) ──► view tree ──► validate ─► layout (taffy)
                                       ─► text  (Parley: shaping, bidi, fallback)
                                       ─► paint (Vello on wgpu)
                                       ─► hit rects + AccessKit tree
                                       ─► desktop-layer surface
```

Every one of those steps happens once, in one place, for every widget. That is
why a widget cannot get text shaping, right-to-left layout, fractional scaling
or screen-reader support wrong: it never does any of them.

## You describe, the host decides

There are no colours, fonts, sizes or coordinates in the widget API. You name
what something **is**, and the theme decides what it **looks like**:

| You write | The host decides |
|---|---|
| `Role::Display`, `Role::Title`, `Role::Caption` … | font, size, weight, line height |
| `Accent::Critical`, `Accent::Positive`, `Accent::Brand` … | an Oklch colour legible against *this* wallpaper at *this* contrast |
| `.muted()` | a lower-emphasis tone of the same colour |
| `.gap(2)`, `.padding(3)` | a step on the theme's spacing scale |

The result: twenty widgets by twenty authors look like one product, and every
one of them follows dark mode, high contrast and font scaling without a line of
code. The [golden images](./gallery.md#what-they-look-like) are the
same widget rendered in three themes, with nothing in the widget changed.

The vocabulary is deliberately small — containers (`panel`, `card`, `row`,
`column`, `group`) and content (`text`, `chip`, `icon`, `divider`, `spacer`,
`progress`, `sparkline`, `image`). See the [API reference](../reference/api.md#building-a-view).

## The wake model

This is where the performance lives. There is **no idle loop and no idle
frame**. Every call into a widget returns a `Wake`:

```rust
pub enum Wake {
    Never,              // only external events: a click, a result, a setting
    At(SystemTime),     // a wall-clock instant — a clock's top of the minute
    After(Duration),    // a delay — a poller's interval
    Animating,          // the display's refresh rate, while something moves
}
```

The scheduler sleeps until the earliest wake across all widgets and blocks on
its event sources with no timer at all in between. Three rules keep it honest:

* **Wakes coalesce.** Widgets asking for the same instant share one wake-up:
  eight clocks cost one, not eight. Prefer `Wake::at_next_multiple_of(60,
  now_seconds())` over `Wake::After(60 s)` — an instant is right across a
  suspend, a time-zone change and an NTP step; a delay drifts.
* **An unchanged view costs nothing.** If `view` returns the same tree as last
  time, nothing is laid out, painted or submitted.
* **Nobody polls.** Anything that takes time — HTTPS, a command — runs off the
  wake path. You get a token now and an event when it finishes.

## The sandbox boundary

A widget is a `wasm32-wasip2` **component** instantiated by Wasmtime. WASI
Preview 2 grants **no ambient authority**: the component has no files, sockets
or environment of its own. What it can reach is uniwidged's host functions,
and every one of them checks the grant on every call — an ungranted call
answers `NotGranted` and does nothing.

What a widget can reach is the list in its manifest, approved by the user:

```toml
capabilities = ["timer", "net:api.open-meteo.com", "fs:cache"]
```

Every capability names *what* — a host, a folder, a command — because that is
something a person can agree to. Around the capabilities sit hard limits:

| Limit | Value |
|---|---|
| Time per call | 16 ms, then interrupted mid-instruction |
| Memory | `memory-mb` from the manifest, enforced by the allocator |
| Failures | three in a row, then the widget is suspended showing its last frame |
| View tree | 1024 nodes, 4 KiB per string, 256 points per sparkline |

A widget that hangs, panics or runs out of memory loses its instance; the
desktop does not notice. A fresh instance starts from `Default::default()`,
which is why **a widget's state should be derived from its data**. See
[Capabilities](./capabilities.md) and the [security model](../project/security.md).

## Trust before the sandbox

The sandbox answers *what can this code do?* A signature answers the question
before it: *who wrote it, and is this what they published?*

* Widgets are signed with **Ed25519**, and the signature covers the component
  **and** the manifest — so nobody can widen what a widget may do and keep a
  signature its users recognise.
* This machine's `trusted-keys` file is the whole trust model. It is plain text
  and meant to be read.
* A registry index is signed too, by a *different* key: the key that says who
  publishes the list. Downloading from a registry is not trusting; your own
  keys still decide.

## Putting it together

```text
 install ─► signature checked against trusted-keys
         ─► manifest shown: "this widget wants to …"
         ─► recorded in config.toml with what you granted

 run     ─► instance per widget, policy = granted ∩ declared
         ─► abi-version checked (9 … host)
         ─► start(cx) once, then view(cx) ⇄ on_event(cx), woken by Wake
```

Next: [Running the desktop](./desktop.md), or straight to
[writing a widget](./writing-a-widget.md).
