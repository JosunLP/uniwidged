# uniwidged — Architecture

uniwidged is a **desktop widget framework**: a host process that renders many
small, always-on panels onto the desktop — below every ordinary window, above
the wallpaper — and a sandboxed SDK third parties write those panels against.

It is the union of two existing projects. [Ephemeris](https://github.com/JosunLP/Ephemeris)
worked out how a window becomes a *desktop widget* on four platforms and is the
source of `uniwidged-shell`. [Ferrum UI](https://github.com/j-pfalzgraf/ferrumui)
is a GPU-rendered UI stack on Vello/wgpu/Parley/taffy/AccessKit and supplies
everything above the surface. What is genuinely new is the seam between them —
a raw desktop-layer surface handed to wgpu instead of to Cairo — and the WASM
boundary that lets somebody else write a widget.

---

## 1. What this is, and what it is not

**Is.** A single host process owning one GPU device and N desktop surfaces. A
widget is a WASM component that returns a description of what it wants shown;
the host lays it out, shapes its text, paints it, hit-tests it and exposes it
to screen readers.

**Is not.** Not a general application framework — that is Ferrum UI, which
uniwidged depends on. Not a wallpaper engine or a shader playground. Not an
HTML/CSS/JS runtime. Not a panel or status-bar replacement, though the shell
layer would serve one.

## 2. Design principles

1. **The host owns the pixels.** A widget never draws. It emits a semantic view
   tree; the host decides what that looks like. Twenty widgets that could each
   paint would look like twenty different toolkits, and the one thing a desktop
   full of widgets must have is a single visual voice.
2. **Nothing runs when nothing changes.** There is no idle loop and no idle
   frame. The scheduler sleeps until the earliest thing any widget asked to be
   woken for. This is the whole performance story; everything else is detail.
3. **Platform code at the edges only.** `uniwidged-shell` is the only crate
   that knows an OS exists. Everything else compiles and behaves identically,
   and is testable without a display.
4. **A widget cannot hurt the desktop.** No ambient authority, a declared
   capability list, a memory cap, and a deadline on every call into guest code.
   A widget that hangs is suspended showing its last frame; the desktop does
   not notice.
5. **Quality is verified, not asserted.** Every visual claim in §7 has a
   golden-image test behind it, run headlessly in CI on a software rasterizer.

## 3. The crate stack

```
                        ┌───────────────────────────────┐
   widget author ─────► │ uniwidged-guest   (WASM SDK)  │  → wasm32-wasip2
                        └───────────────┬───────────────┘
                                        │  WIT component interface
 ═══════════════════════════════════════╪═══════════════════ sandbox boundary
                                        │
                        ┌───────────────▼───────────────┐
                        │ uniwidged-runtime  (Wasmtime) │  instantiate · capabilities
                        │                               │  memory cap · epoch deadline
                        └───────────────┬───────────────┘
                                        │  View  (pure data, no OS, no GPU)
                        ┌───────────────▼───────────────┐
                        │ uniwidged-view                │  nodes + style tokens
                        └───────────────┬───────────────┘
                                        │
  ┌─────────────────┐   ┌───────────────▼───────────────┐
  │ uniwidged-host  │◄─►│ uniwidged-compose             │  taffy layout · Parley text
  │  scheduler      │   │   → Scene · hit rects · a11y  │  → vello::Scene
  │  config · IPC   │   └───────────────┬───────────────┘
  │  capability     │                   │  vello::Scene
  │  policy         │   ┌───────────────▼───────────────┐
  └────────┬────────┘   │ uniwidged-render   (wgpu)     │  1 Device · N surfaces
           │            │                               │  1 submit per frame
           │            └───────────────┬───────────────┘
           │                            │  raw-window-handle
           │            ┌───────────────▼───────────────┐
           └───────────►│ uniwidged-shell               │  desktop-layer surfaces
                        │  wayland │ x11 │ win32 │ appkit│  + input + configure + DPI
                        └───────────────┬───────────────┘
                                        │
                        ┌───────────────▼───────────────┐
                        │ uniwidged-core                │  geometry · color · ids
                        │                               │  events · wake
                        └───────────────────────────────┘
```

| Crate | Owns | Depends on an OS? | Needs a GPU? |
|---|---|---|---|
| `uniwidged-core` | geometry, colour, ids, input events, the wake model | no | no |
| `uniwidged-view` | the view tree and style tokens that cross the sandbox | no | no |
| `uniwidged-compose` | view → layout → text → `vello::Scene`, hit rects, a11y nodes | no | no |
| `uniwidged-render` | the wgpu device, per-surface renderers, readback presentation | no | yes |
| `uniwidged-shell` | desktop-layer surfaces, input, configure, scale, menus | **yes** | no |
| `uniwidged-runtime` | Wasmtime, capability enforcement, resource limits | no | no |
| `uniwidged-guest` | the SDK widget authors compile against | no | no |
| `uniwidged-host` | the daemon: scheduler, config, policy, lifecycle | no | no |
| `uniwidged-cli` | `uniwidged run · new · install · doctor · snapshot` | no | no |

Two of these compile to WASM (`uniwidged-core`, `uniwidged-view`) because the
guest SDK shares their types with the host. That is deliberate: the view tree a
widget builds and the view tree the host lays out are the *same Rust type*, not
two definitions that must be kept in sync.

## 4. Data flow of one frame

```
 scheduler wakes            (a timer is due · input arrived · a fetch finished)
      │
      ▼
 for each widget marked dirty:
      runtime.view(id)  ──► guest `view()` runs under a deadline
                       ◄──  View tree
      │
      ▼
 diff View against the last one
      │  unchanged → the surface stays clean, no frame is built at all
      ▼
 compose:  View ──► taffy layout ──► Parley shape/measure ──► vello::Scene
                                 └──► absolute hit rects (for input)
                                 └──► AccessKit nodes  (for screen readers)
      │
      ▼
 render:   per dirty surface: Vello compute pass → intermediate texture
           then ONE CommandEncoder blits every one of them onto its
           swapchain image, and submits once
             ← clean surfaces are not encoded, not submitted, not presented
      │
      ▼
 present each dirty surface
      │
      ▼
 next_wake = min over all widgets     ──► sleep
```

Input runs the other way: `uniwidged-shell` turns a native event into a
`uniwidged_core::PointerEvent`/`KeyEvent`, the host hit-tests it against the
recorded rects of that surface, and delivers a semantic event (`clicked(node)`,
`hovered(node)`) into the guest — never raw coordinates it would have to
interpret itself.

## 5. The wake model

This is where the performance requirement actually lives. Ephemeris costs
0.03 s of CPU per 40 s of idle, and it does so by having no idle loop: three
timers, each alive only while something needs it. uniwidged generalises that
to N widgets.

```rust
pub enum Wake {
    /// Only external events — input, a capability callback, a config change.
    Never,
    /// A wall-clock instant. A clock asks for the top of the next minute.
    At(SystemTime),
    /// A delay from now. A poller asks for its interval.
    After(Duration),
    /// The display's refresh rate, while something is genuinely moving.
    Animating,
}
```

Every call into a widget returns one. The scheduler sleeps until the earliest
across all of them, and the host submits **no frame** in between.

The rules that keep this honest:

* **No widget may poll.** A capability that waits — an HTTP fetch, a file
  watch, a D-Bus signal — runs on the host's async runtime, off the wake path,
  and wakes its widget when it has something. A widget never spins to find out.
* **`Animating` is auto-clearing.** It is granted for the duration of an
  animation the host is driving. A widget that returns `Animating` across many
  frames with a byte-identical view tree is logged, and then throttled.
* **The clock aligns.** Widgets asking for "the next minute" are coalesced onto
  the same wake, so eight clocks cost one wake-up, not eight.

**Budget.** With eight widgets installed and none animating, the target is
under 0.1 % CPU averaged over a minute, zero frames submitted, and the GPU at
its idle clock. Regressions are caught by a soak test, not by hoping.

## 6. The sandbox boundary

### What crosses it

The guest returns a **view tree**: nodes with a role, a style token, text or an
image handle, and children. It is pure data — a few hundred bytes to a few
kilobytes — and it is the *only* thing that comes back.

It crosses **flat**, as an arena: a list of nodes, each naming its children by
index, with node 0 as the root. WIT has no recursive types, so a record cannot
contain a list of itself and there was no choice about this — but it turns out
to be the better shape anyway. One allocation per side instead of one per node,
and the host can check the whole structure before touching any of it.

The check matters, because the arena arrives from a sandbox. The rule that does
the work is that **a child's index must be greater than its parent's**: an
arena must be topologically ordered, which makes a cycle impossible to *express*
rather than merely detected. There is no visited set to get wrong and no depth
at which the check gives up — and a recursive decoder would have discovered a
cycle by running out of stack, which on a desktop process means the widget takes
the desktop with it. Two parents, an index out of range, an unattached node, too
many nodes and an over-long string are refused alongside it. See
`uniwidged_view::wire`.

The guest never sees a pixel, a font, a GPU, a window, or a coordinate it did
not ask for. Layout, text shaping, painting, hit-testing and the accessibility
tree are all the host's.

Four reasons, in order of weight:

1. **One visual voice.** Style is named, not specified: a widget asks for
   `Style::Title`, not for 15 px semibold #E8E8EA. The theme resolves it. This
   is what makes a screen full of third-party widgets look like one product.
2. **Text is done once, well.** Shaping, bidi, font fallback and hinting are
   the hardest and heaviest part of a UI stack. Every widget shipping its own
   copy would be larger *and* worse.
3. **The ABI stays small.** A view tree is kilobytes. A pixel buffer would be
   megabytes per frame across a sandbox boundary — the thing that makes every
   web-technology widget framework cost what it costs.
4. **Accessibility comes free.** The host knows the semantic tree, so AccessKit
   nodes fall out of compose with no effort from the widget author.

### Capabilities

A widget declares what it needs; the user approves it at install; the runtime
enforces it. WASI Preview 2 has no ambient authority, so anything undeclared is
simply absent rather than denied.

| Capability | Grants |
|---|---|
| `timer` | returning a `Wake` other than `Never` |
| `net:<host>` | HTTPS to exactly those hosts, through the host's client |
| `fs:<subpath>` | read/write inside the widget's own data directory |
| `secrets` | the OS keychain, namespaced to this widget |
| `locale` | the user's locale-formatted dates, times and numbers |
| `measure` | measuring a string in a named style, for its own truncation calls |
| `command:<name>` | running one declared external command |
| `metrics` | CPU, memory, battery, network counters |

`net` is host-scoped rather than a blanket switch because "this weather widget
talks to `api.open-meteo.com`" is a sentence a user can consent to, and
"this widget uses the network" is not.

### Containment

* **Memory** — a `ResourceLimiter` cap per instance, declared in the manifest
  and visible to the user.
* **Time** — epoch-based interruption with a deadline on every call. A widget
  that blows it is suspended, marked errored in the UI, and keeps showing its
  last frame. **The desktop never blocks on guest code.**
* **State** — its own data directory, its own keychain namespace, no path out.

## 7. Graphical quality, concretely

"High graphical quality" is a list of commitments with tests behind them, not
an aspiration:

* **Analytic anti-aliasing** on every vector edge, via Vello's compute
  rasterizer. No MSAA fringing, no stair-stepped rounded corners.
* **True per-pixel alpha** against the real wallpaper on all four platforms —
  DirectComposition, `CAMetalLayer`, an ARGB Wayland surface, a 32-bit X11
  TrueColor visual. This is the difference between a widget that sits *on* the
  desktop and one that sits *in front of* a grey rectangle.
* **Gamma-correct compositing.** Gradients and shadows are interpolated in
  premultiplied linear space. (Ephemeris's `canvas.rs` documents exactly the
  grey-haze artefact that straight interpolation produces between two partly
  transparent stops; Vello does not have it.)
* **Perceptual colour.** Theme colours are authored in Oklch and resolved at
  paint time, so a lightness ramp looks even rather than measuring even. sRGB
  by default, Display P3 where the surface reports it.
* **Fractional scaling handled once**, at the surface, as an `Affine::scale`
  into the presented scene. No widget is DPI-aware; nothing is bitmap-scaled.
* **Text through Parley** — full shaping, bidi and font fallback, hinted and
  gamma-corrected at the real device scale.
* **Real Gaussian drop shadows**, not stacked translucent strokes. The surface
  is larger than the visible glass by the shadow's extent on every side, and
  that margin is `uniwidged-compose`'s to own — a widget author cannot know the
  theme's shadow, and the M0 spike clipped its own shadow by getting this
  wrong.
* **Backdrop blur where the platform gives it** — DWM `SystemBackdrop`,
  `NSVisualEffectView`, KWin/`wp_blur` — with a drawn fallback that degrades
  visibly rather than silently.

Each of these is pinned by a golden-image snapshot test rendered headlessly on
lavapipe, so a regression fails CI rather than shipping.

## 8. Platform matrix

The behaviours that define a desktop widget, and how each platform provides
them. Everything in the first four rows is adopted from Ephemeris; the last row
is new work.

| | Windows | macOS | Wayland | X11 |
|---|---|---|---|---|
| **below windows, above desktop** | `HWND_BOTTOM` forced in `WM_WINDOWPOSCHANGING` | `kCGDesktopIconWindowLevel + 1` | `wlr-layer-shell`, `BOTTOM` layer | `_NET_WM_STATE_BELOW` |
| **never takes focus** | `WS_EX_NOACTIVATE` | `canBecomeKeyWindow → NO` | `keyboard-interactivity = none` | `WM_HINTS.input = False`, no `WM_TAKE_FOCUS` |
| **no taskbar / no Alt-Tab** | `WS_EX_TOOLWINDOW` | `.ignoresCycle` + `.accessory` policy | (the layer has neither) | `_NET_WM_STATE_SKIP_TASKBAR`, `_SKIP_PAGER` |
| **on every desktop / Space** | (bottom-most, always present) | `.canJoinAllSpaces` + `.stationary` | (the layer is per-output) | `_NET_WM_STATE_STICKY` |
| **per-pixel alpha** | `WS_EX_NOREDIRECTIONBITMAP` + DirectComposition | borderless, `opaque = NO`, `hasShadow = NO` | ARGB8888 surface | 32-bit TrueColor visual + own colormap |
| **wgpu attaches via** | `SurfaceTargetUnsafe::CompositionVisual` (DX12) | `SurfaceTargetUnsafe::CoreAnimationLayer` | `RawWindowHandle::Wayland` | `RawWindowHandle::Xlib` — **see §10.1** |

Two notes carried over from Ephemeris, because both were hard-won:

* **`_NET_WM_WINDOW_TYPE_DESKTOP` is the wrong answer on X11.** It means *is*
  the desktop, and window managers that honour it stack the widget beneath the
  desktop icons and stop delivering clicks. `_NET_WM_STATE_BELOW` on an
  ordinary window says what is actually wanted.
* **GNOME's Mutter has no layer shell and has said it will not add one.**
  There the surface falls back to an `xdg_toplevel`: it sits among the windows
  rather than behind them, and it cannot place itself, because Wayland gives no
  client that power. Both facts go in the log at start-up. Degrading quietly
  was the wrong answer in Ephemeris and is the wrong answer here — a widget
  that stops doing the one thing it is for should say which.

There is a fifth target, **headless**, which renders to a texture and reads
back a PNG. It is not a courtesy: it is how every visual test in CI runs.

## 9. Testing strategy

* **Golden-image snapshots** — fixed view trees rendered through compose and
  render, diffed against committed PNGs with a tolerance. Runs on lavapipe, so
  CI needs no GPU and no display. Regenerate intentional changes with
  `UNIWIDGED_UPDATE_SNAPSHOTS=1`.
* **View-tree unit tests** — pure data, no GPU: layout maths, diffing, hit-rect
  computation, a11y tree shape, style resolution.
* **Runtime conformance suite** — one test widget that attempts every
  capability, run under a deny-all policy, asserting each attempt fails. A
  sandbox that is not tested for what it *refuses* is not tested.
* **Scheduler soak** — eight idle widgets for five minutes; assert zero frames
  submitted and the CPU budget of §5 held.
* **Shell tests** — the Wayland path against a nested headless compositor
  (`cage` / `weston --backend=headless`) in CI. Windows, macOS and X11 shells
  are exercised on their own runners.

## 10. Known risks

### 10.1 X11 per-pixel alpha with a Vulkan swapchain — the highest risk

Ephemeris gets a transparent X11 window by picking a 32-bit TrueColor visual
and drawing into it with Cairo. A Vulkan swapchain on X11 usually advertises
only `VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR`, which would put an opaque rectangle
where the wallpaper should be. Mitigations, in the order they will be tried:

1. Query for `PRE_MULTIPLIED` / `INHERIT` composite-alpha support and use it
   where the driver offers it.
2. An EGL/GL path with an ARGB visual — EGL on X11 does honour the visual's
   alpha channel.
3. **Readback presentation.** Vello renders to an offscreen texture exactly as
   it does everywhere else; the pixels are read back and handed to XRender or
   `XShmPutImage`, which is Ephemeris's own presentation path. Rasterisation is
   untouched — the same `vello::Scene`, the same GPU, the same output — so the
   golden images still apply. Only *presentation* falls back, which is the
   thing X11 is actually bad at.

   This was originally written as "`vello_cpu` from the same scene", and that
   is not possible: `vello` 0.9 builds its scene on `vello_encoding`, while
   `vello_cpu` 0.2 builds one on `vello_common`. They are disjoint types, and a
   scene cannot cross between them. A true no-GPU-at-all path would need
   `uniwidged-compose` to emit into both, and is not planned until the two
   Vello lines converge.

**This was spiked before anything was built on top of it.** On Wayland the
answer is unambiguous: the surface offers `[Opaque, PreMultiplied]`, and
`PreMultiplied` composites correctly through the whole Vello → wgpu →
layer-surface path (`spikes/m0_layer_shell/FINDINGS.md`). X11 remains open and
is settled in milestone M4.

If none of the three works there, X11 gets the CPU path permanently and the
architecture is unchanged — which is precisely why compose emits a `Scene`
rather than talking to wgpu.

### 10.2 wgpu does not expect somebody else to drive the surface — **resolved**

A `wlr-layer-shell` surface must `ack_configure` before its first commit, and a
DirectComposition visual is presented through a composition tree the window
does not own. wgpu's surface API looks like it assumes it owns configure and
present timing.

**Measured on 2026-08-27, and it does not.** The M0 spike drives the handshake
itself — create the surface, give it the layer role, commit with no buffer,
receive `configure`, `ack_configure` — and only then hands the surface to wgpu.
wgpu accepts a surface somebody else configured without complaint. See
`spikes/m0_layer_shell/FINDINGS.md`.

One correction came out of it: **wgpu 29 wants the display handle at `Instance`
creation**, not only at surface creation
(`InstanceDescriptor::new_with_display_handle`). Vulkan ignores it; the GL
backend needs it to bind EGL to that `wl_display`. Since GL is the second of
the three X11 mitigations in §10.1, `uniwidged-shell` supplies it from the
start.

### 10.3 GPU cost at idle

One wgpu device plus per-surface swapchains is roughly 30–60 MB of VRAM and a
driver context that Ephemeris's 4 MB RSS does not pay. This is the deliberate
price of §7, and it is paid **once for all widgets** rather than per widget —
which is the entire reason for a single host process. If it proves too high on
laptops, the device can be dropped after a long idle with nothing animating and
rebuilt on the next wake; the surfaces keep their last frame meanwhile.

### 10.4 The component model is still moving

WASI Preview 2 and the component model are stable enough to build on but not
frozen. Wasmtime is pinned to an exact version, the WIT world is vendored in
`wit/`, and the guest ABI gets a version field checked at instantiation —
before `start`, and before anything a widget could do about it.

Two of its constraints already shaped the design rather than merely annoying
it: **no recursive types**, which is why the view tree crosses flat (§6, and
the better shape for it), and **a trapped instance cannot be re-entered**,
which is why a deadline overrun ends an instance instead of being retried.

## 11. Provenance and licensing

uniwidged is MIT OR Apache-2.0. `uniwidged-shell` derives from Ephemeris
(GPL-3.0-or-later) under a relicence by its sole copyright holder; see
[`NOTICE.md`](NOTICE.md) for the verification. This is not a detail: a
framework's licence is the licence its users inherit, and a permissive one is
what lets a widget author choose their own.
