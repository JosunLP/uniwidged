# uniwidged

**A cross-platform desktop widget framework, built for graphical quality and
for costing nothing when nothing is happening.**

One host process owns one GPU device and every widget's surface. Widgets sit
below your windows and above your wallpaper, composited per-pixel against it —
not floating in a grey rectangle. They are WASM components: sandboxed,
installable at runtime, and written in any language that targets the component
model.

> **Status: scaffold, with the load-bearing assumption proven.** The
> architecture is settled and written down, the vocabulary crates are real and
> tested, and the platform layers are documented skeletons.
>
> **Milestones M0 through M3 are done.** A `wlr-layer-shell` surface on the bottom
> layer really does back a wgpu swapchain with per-pixel alpha, and the widget
> really does stay below ordinary windows. `uniwidged-render` drives it: one
> device, many surfaces, an idle frame costing zero submits, and a headless
> render that matches the on-screen one to within one 8-bit unit.
> `uniwidged-compose` turns a view tree into that scene — layout, text, theme,
> icons, hit rectangles and an accessibility tree, all from one pass — with
> fifteen golden images behind it. And widgets really are sandboxed WASM
> components: `widgets/conformance` is built for `wasm32-wasip2`, instantiated
> by the real runtime, and thirteen tests assert what the sandbox *refuses*.
> All measured, not assumed; see
> [`spikes/m0_layer_shell/FINDINGS.md`](spikes/m0_layer_shell/FINDINGS.md).
> Next is M4, X11. See [`ROADMAP.md`](ROADMAP.md).

![A widget on the bottom layer, with the wallpaper showing through it](spikes/m0_layer_shell/images/widget.png)

*Not a mockup: `grim`'s capture of a headless sway session, showing a Vello
scene on a layer surface composited per-pixel against the wallpaper.*

## Where it comes from

uniwidged is the union of two projects, and both halves are load-bearing.

[**Ephemeris**](https://github.com/JosunLP/Ephemeris) worked out how a window
becomes a *desktop widget* rather than an application window — on Windows,
macOS, Wayland and X11, with all the flags and protocols that implies, and all
the ways each platform gets it wrong. That is `uniwidged-shell`.

[**Ferrum UI**](https://github.com/j-pfalzgraf/ferrumui) is a GPU-rendered UI
stack on Vello, wgpu, Parley, taffy and AccessKit. That is everything above the
surface.

What is new is the seam between them — handing a desktop-layer surface to wgpu
instead of to Cairo — and the sandbox boundary that lets somebody else write a
widget.

## What makes it fast

There is no idle loop and no idle frame. Every widget answers "when should I be
called again?", the scheduler sleeps until the earliest of those, and between
them the process blocks on its event sources with no timer at all. Widgets
whose deadlines fall close together share one wake-up — eight clocks cost one,
not eight — and a widget whose view tree has not changed produces no GPU work
whatsoever.

Ephemeris reached 0.03 s of CPU per 40 s of idle this way with three hand-rolled
timers. uniwidged generalises it to N widgets that have never heard of each
other.

## What makes it look right

Analytically anti-aliased vector rendering, gamma-correct compositing,
perceptual colour in Oklch, real Gaussian shadows, full text shaping with bidi
and font fallback, fractional scaling resolved once at the surface, and true
per-pixel alpha against the actual wallpaper on all four platforms.

Every one of those has a golden-image test behind it, rendered headlessly in CI
on a software rasterizer. See [`ARCHITECTURE.md`](ARCHITECTURE.md) §7.

## Writing a widget

You describe; the host decides what it looks like. That is what makes a screen
full of widgets by different authors look like one product.

```rust
use uniwidged_guest::prelude::*;

#[derive(Default)]
struct Clock;

impl Widget for Clock {
    fn view(&mut self, cx: &Cx) -> View {
        let now = now_seconds();
        panel(NodeId::named("clock"))
            .child(text(cx.format_time(now).unwrap_or_default()).role(Role::Display))
            .child(text(cx.format_date(now).unwrap_or_default()).role(Role::Caption).muted())
            .view()
    }

    // The top of the next minute — not "in 60 seconds", so the clock stays
    // right across a suspend, a timezone change and an NTP step.
    fn wake(&self) -> Wake { Wake::At(next_minute()) }
}

uniwidged_guest::export!(Clock);
```

Every host call returns a `Result`: a user can decline anything, and a widget
that disappears when they do is a worse answer than one that shows its last
value and says it is stale. Off-target the whole thing still compiles and its
tests still run — `export!` expands to nothing there — so a widget's logic can
be tested without a WASM toolchain.

No colours, no fonts, no pixels, no coordinates. A widget names
`Role::Display`; the theme decides what that is. A widget names
`Accent::Critical`; the theme picks a red that is legible against *this*
wallpaper at *this* contrast setting.

Capabilities are declared and host-scoped:

```toml
# widget.toml
name = "weather"
capabilities = ["timer", "net:api.open-meteo.com", "fs:cache"]
memory-mb = 32
```

`net:api.open-meteo.com`, not `network` — because "this widget talks to
api.open-meteo.com" is something a user can agree to, and "this widget uses the
network" is not.

## Layout

```
crates/
  uniwidged-core       geometry · colour · ids · events · the wake model
  uniwidged-view       the view tree that crosses the sandbox boundary
  uniwidged-compose    view → layout → text → Vello scene · hit rects · a11y
  uniwidged-render     one wgpu device, many surfaces, swapchain or readback
  uniwidged-shell      desktop-layer surfaces: wayland · x11 · win32 · appkit
  uniwidged-runtime    the WASM sandbox: capabilities, limits, lifecycle
  uniwidged-guest      the SDK widget authors compile against
  uniwidged-host       the daemon: scheduler, configuration, policy
  uniwidged-cli        `uniwidged run · new · install · list · doctor`
wit/uniwidged.wit      the host/guest contract
widgets/conformance    a widget that tries everything, so the sandbox can be
                       tested for what it refuses
spikes/m0_layer_shell  the proof that the whole thing is possible
```

## Building

```sh
git clone https://github.com/j-pfalzgraf/ferrumui  ../ferrumui   # a sibling checkout
cargo test --workspace
```

`uniwidged-compose` depends on `ferrum-layout` by path while Ferrum UI is
unpublished — a sibling checkout is expected, and the dependency becomes a
version requirement the moment it reaches crates.io.

Some tests need more than a compiler, and say so rather than skipping quietly:

| | |
|---|---|
| the golden images | a Vulkan driver — `mesa-vulkan-drivers` supplies a software one. Without it they skip; `UNIWIDGED_REQUIRE_GPU=1` turns that into a failure, and CI sets it. |
| the sandbox conformance suite | `rustup target add wasm32-wasip2`. It builds the conformance widget itself, so there is nothing to skip. |
| the M0/M1 spike | a wlroots compositor and `grim`; `spikes/m0_layer_shell/run.sh` starts a headless sway if none is running. |

## Licence

MIT OR Apache-2.0, at your option — deliberately, because a framework's licence
is the licence its users inherit, and a widget author should be free to choose
their own. The shell layer derives from Ephemeris (GPL-3.0-or-later) under a
relicence by its sole copyright holder; the verification is in
[`NOTICE.md`](NOTICE.md).
