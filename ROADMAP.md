# uniwidged — Roadmap

Milestones are ordered by **risk first, then by dependency**. The one thing
that could invalidate the architecture is spiked before anything is built on
it; after that the order is whatever unblocks the most.

Each milestone is done when its **exit criterion** holds. "Written" is not an
exit criterion.

---

## M0 · Prove the seam — `wlr-layer-shell` + wgpu ✅ **done, 2026-08-27**

The whole design rests on one unproven claim: that a desktop-layer surface can
back a GPU swapchain with per-pixel alpha. Nobody should write another line of
the framework until that is a running binary.

* Connect to a Wayland display, bind `zwlr_layer_shell_v1`, create a layer
  surface on the `BOTTOM` layer with `keyboard-interactivity` off.
* Drive the configure/`ack_configure` handshake from our own code, then attach
  a `wgpu::Surface` created from `RawWindowHandle::Wayland`.
* Render one Vello scene — a rounded translucent panel with a drop shadow and a
  line of text — and present it.
* Confirm `VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR`, and that the wallpaper
  is visible through the panel.
* Detect the absence of the layer shell and fall back to `xdg_toplevel`, saying
  so in the log.

**Exit — met.** `spikes/m0_layer_shell/` does all five, with two screenshots and
two arithmetic checks behind them: the wallpaper measurably shows through the
glass (`rgb(25, 43, 72)` where the wallpaper is `rgb(30, 111, 217)`), and an
opaque window covers the widget completely. §10.2 is resolved; §10.1 is
resolved for Wayland and stays open for X11 until M4. Full write-up in
`spikes/m0_layer_shell/FINDINGS.md`.

## M1 · `uniwidged-render` — one device, many surfaces ✅ **done, 2026-08-27**

* One `wgpu::Device` shared by every surface; per-surface swapchain,
  intermediate storage texture and `TextureBlitter`.
* Dirty tracking: clean surfaces are not rendered, not encoded and not
  presented. Every dirty surface's blit goes onto one `CommandEncoder`,
  finished with one `queue.submit`. (Vello 0.9 submits its own compute pass per
  `render_to_texture` call and offers no way to pass in an encoder, so a frame
  costs one submit per *dirty* surface plus one for all the blits together —
  not the single submit originally planned. Measured, not assumed.)
* Two presentation modes behind one interface: a wgpu swapchain, and readback —
  render to a texture, read the pixels back, let the platform composite them.
  Readback is what the X11 mitigation and every headless test use.

**Exit — met.** Twelve tests on a software Vulkan device, including three
surfaces of different sizes rendering different scenes in one `render` call
with each getting its own content, an idle frame costing zero submits, and
undrawn pixels staying fully transparent.

The headless-matches-on-screen half is checked against a real compositor by
`spikes/m0_layer_shell/compare.py`, which renders one scene down both paths and
compares them pixel for pixel: **worst channel difference 1, mean 0.272**. That
comparison found a genuine bug — Vello emits straight alpha where a Wayland
swapchain expects premultiplied, so every translucent pixel was too bright on
screen and nothing looking at one path alone could see it. Written up in
`spikes/m0_layer_shell/FINDINGS.md`.

`vello_cpu` is *not* part of this milestone, and cannot be: see §10.1.

## M2 · `uniwidged-compose` — view tree to pixels ✅ **done, 2026-08-27**

* `View` → taffy layout (via `ferrum-layout`), with the `gap`/`padding` theme
  steps resolved to pixels.
* Text through Parley (via `ferrum-text`): shaping, bidi, fallback, ellipsis,
  optional wrapping.
* The theme: `Role` → size/weight/opacity, `Accent` → Oklch → the surface's
  colour space. Light, dark and high-contrast.
* Vello scene emission: rounded rects, gradients, real Gaussian shadows, the
  drawn icon set.
* Hit rects and AccessKit nodes emitted from the *same* pass — see
  `ARCHITECTURE.md` §4 on why they may never be computed separately.

**Exit — met.** Fifteen golden images in
`crates/uniwidged-compose/tests/snapshots/`, covering every role, every semantic
accent, the categorical palette, the icon set, both appearances, high contrast,
truncation and wrapping, Arabic in both directions, and 100 %/150 %/200 %
scaling. They render through the whole stack on a software Vulkan device and are
made deterministic by a bundled font, so a colour, weight, radius or shadow that
changes fails CI instead of shipping.

Text is Parley directly rather than through `ferrum-text`: that wrapper exposes
only size and a generic family, and two of the §7 commitments need more — font
weight (most of what makes a hierarchy readable, and *all* of it at high
contrast, where opacity is gone) and tabular figures (a clock in proportional
digits changes width as the minute turns). `ferrum-layout` is used unchanged.

72 unit tests besides the images, including the invariant that no colour the
theme can produce falls outside the sRGB gamut — several did on the first
attempt, and a silently clipped palette is one whose author did not choose what
shipped.

## M3 · `uniwidged-runtime` + `uniwidged-guest` — the sandbox ✅ **done, 2026-08-27**

* Generate host and guest bindings from `wit/uniwidged.wit`.
* Wasmtime: instantiate, `ResourceLimiter` memory cap, epoch interruption at
  the call deadline, suspend after repeated overruns.
* Wire the capability policy — already written and tested — to the host
  functions.
* `uniwidged_guest::export!` and the manifest format.
* The conformance widget: attempts every capability, run under deny-all.

**Exit — met.** `widgets/conformance` is a real component built for
`wasm32-wasip2`, instantiated by the real runtime, producing a view the host
accepts. Thirteen conformance tests, of which most are about what is refused:
every capability under a deny-all policy, a `file://` URL refused as *malformed*
rather than ungranted, a path escape refused even where the directory is
granted, and a host grant that does not extend to a name merely containing it.

The suite **builds the widget itself** rather than looking for a prebuilt file:
one that skips when an artefact is missing eventually always skips, and nobody
notices until it matters.

Two findings changed the design. WIT has **no recursive types**, so the view
tree crosses as a validated flat arena — see `ARCHITECTURE.md` §6, where the
ordering rule makes a cycle impossible to express rather than merely detected.
And a **trapped component instance cannot be re-entered**: the first design
counted deadline overruns per call and expected a widget to survive two, and
the second call came back with "cannot enter component instance" in 95
microseconds. A trap now ends the instance, recovery is a fresh one, and
`a_fresh_instance_recovers_a_widget_that_died` proves that path works.

Both halves of containment are tested against a widget that really misbehaves:
one that never returns is interrupted at its deadline, and one that allocates
in a loop is stopped by the memory cap rather than by the clock.

## M4 · X11 ✅ **done, 2026-08-27**

* The window: 32-bit TrueColor visual, `_NET_WM_STATE_BELOW`, `SKIP_TASKBAR`,
  `SKIP_PAGER`, `STICKY`, `WM_HINTS.input = False`, Motif decorations off.
* The `poll` loop, waiting on the display socket and the host's wake pipe with
  a timeout from `Deadline`.
* The three transparency paths of `ARCHITECTURE.md` §10.1, tried in order, with
  the chosen one reported by `uniwidged doctor`.

**Exit — met, with one path deliberately unbuilt.** `spikes/m4_x11` creates the
window through `uniwidged-shell`, renders a widget composed by
`uniwidged-compose`, and checks the arithmetic: the window on screen is the
scene composited over the wallpaper to within one 8-bit unit, and an ordinary
opaque window covers it. Path 3 is forced with `SPIKE_FORCE_READBACK=1` and
produces the same numbers, which is what makes "only presentation falls back" a
measurement rather than a claim.

Path 2 (EGL with an ARGB visual) is **not** built. It would buy GPU presentation
on a driver that refuses alpha through Vulkan, where readback already gives GPU
rasterisation and correct output; that is a performance difference on hardware
nobody has measured. `AlphaPath` still names it, so the gap is visible rather
than forgotten. Write-up in `spikes/m4_x11/FINDINGS.md`.

The screenshots earned their place twice over: they found two rendering bugs
with a single root cause — taffy rounds a computed box down to whole pixels —
that the golden-image suite could not see, because its bundled font made the
arithmetic land differently.

## M5 · Windows

* `CreateWindowEx` with `WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW |
  WS_EX_NOACTIVATE`, `WS_POPUP`; `HWND_BOTTOM` forced in
  `WM_WINDOWPOSCHANGING`.
* A DirectComposition visual tree; wgpu attached via
  `SurfaceTargetUnsafe::CompositionVisual`.
* Per-monitor-v2 DPI; `DWMWA_SYSTEMBACKDROP_TYPE` for the backdrop.

**Exit:** the widget stays below a maximised window, never steals focus, has no
Alt-Tab entry, and shows the wallpaper through its glass.

## M6 · macOS

* Borderless `NSWindow` subclass with `canBecomeKeyWindow → NO`, level
  `kCGDesktopIconWindowLevel + 1`, `.canJoinAllSpaces | .stationary |
  .ignoresCycle`, `.accessory` activation policy.
* Layer-backed `NSView` with a non-opaque `CAMetalLayer`; wgpu attached via
  `SurfaceTargetUnsafe::CoreAnimationLayer`.
* `NSVisualEffectView` behind it for the backdrop.

**Exit:** as M5, plus: the widget follows the user across Spaces and does not
slide away under Mission Control.

## M7 · The host and the CLI

* Lifecycle: install, configure, enable, disable, remove; per-widget data
  directory and keychain namespace.
* The capability approval flow — what the user is shown before a widget runs.
* Placement: per-output, saved by output *name*, restored on hotplug.
* Dragging, resizing and a lock switch, from Ephemeris's model.
* `uniwidged run · new · install · list · doctor · snapshot`.
* Autostart per platform.
* The idle soak test of `ARCHITECTURE.md` §5, in CI.

**Exit:** eight widgets installed, arranged across two monitors, surviving a
restart; and the soak test holding the CPU budget with zero frames submitted.

## M8 · Shipping

* Packaging: MSI/MSIX, a signed and notarised `.app`, Flatpak, an AUR package,
  a `.deb`.
* A widget registry format, and signature verification on install.
* The authoring guide, an API reference, and four reference widgets — clock,
  weather, system load, calendar — each exercising a different capability.

**Exit:** somebody who has never seen the repository writes and installs a
widget from the guide alone.

---

## Not planned

Named so that nobody has to ask twice:

* **A general application framework.** That is Ferrum UI, which this depends
  on.
* **An HTML/CSS/JS runtime.** The point of §6 of `ARCHITECTURE.md` is that
  widgets do not bring their own renderer.
* **Widgets that draw their own pixels.** Same reason. A `Canvas` node for
  charts is a *maybe*, and would still be host-drawn from a declarative series.
* **A panel or status-bar replacement.** The shell layer would serve one; it is
  somebody else's project.
