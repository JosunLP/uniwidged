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

## M1 · `uniwidged-render` — one device, many surfaces

* One `wgpu::Device` shared by every surface; per-surface swapchain,
  intermediate storage texture and `TextureBlitter`.
* Dirty tracking: only dirty surfaces are encoded, all of them on one
  `CommandEncoder`, finished with one `queue.submit`.
* The headless path: render to texture, read back a PNG.
* `vello_cpu` behind the same `Scene` input, selectable per surface.

**Exit:** three surfaces of different sizes rendering different scenes in one
submit, verified by a frame capture; and the same three scenes rendered
headlessly to PNGs that match the on-screen output.

## M2 · `uniwidged-compose` — view tree to pixels

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

**Exit:** the golden-image suite, covering every `Role`, every `Accent`, both
appearances, high contrast, RTL, and 100 %/150 %/200 % scaling. Every visual
claim in `ARCHITECTURE.md` §7 has an image behind it.

## M3 · `uniwidged-runtime` + `uniwidged-guest` — the sandbox

* Generate host and guest bindings from `wit/uniwidged.wit`.
* Wasmtime: instantiate, `ResourceLimiter` memory cap, epoch interruption at
  the call deadline, suspend after repeated overruns.
* Wire the capability policy — already written and tested — to the host
  functions.
* `uniwidged_guest::export!` and the manifest format.
* The conformance widget: attempts every capability, run under deny-all.

**Exit:** a widget compiled to `wasm32-wasip2`, installed from a file, rendering
on screen; and the conformance suite green, meaning every ungranted capability
is refused rather than merely unused.

## M4 · X11

* The window: 32-bit TrueColor visual, `_NET_WM_STATE_BELOW`, `SKIP_TASKBAR`,
  `SKIP_PAGER`, `STICKY`, `WM_HINTS.input = False`, Motif decorations off.
* The `poll` loop, waiting on the display socket and the host's wake pipe with
  a timeout from `Deadline`.
* The three transparency paths of `ARCHITECTURE.md` §10.1, tried in order, with
  the chosen one reported by `uniwidged doctor`.

**Exit:** the same widget as M3, transparent, below windows, on X11 — under at
least one of the three paths, with the other two exercised by forcing them.

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
