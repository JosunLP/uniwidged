# M0 — Findings

Run on 2026-08-27, Ubuntu 24.04, sway 1.9 (`WLR_BACKENDS=headless`,
`WLR_RENDERER=pixman`), Mesa lavapipe (LLVM 20.1.2), wgpu 29.0.4, Vello 0.9.0.

**Both open questions are answered, and the answer is yes.**

![The widget, with the wallpaper showing through it](images/widget.png)

That is not a mockup. It is `grim`'s capture of the compositor output: a
`zwlr_layer_shell_v1` surface on the bottom layer, rendered by Vello through a
wgpu Vulkan swapchain, composited per-pixel against sway's background.

## §10.1 — Per-pixel alpha with a GPU swapchain

```
adapter: llvmpipe (LLVM 20.1.2, 256 bits) (Vulkan)
swapchain format: Bgra8Unorm
alpha modes offered: [Opaque, PreMultiplied]
alpha mode chosen:   PreMultiplied
```

`PreMultiplied` is offered on Wayland and works. This was the *expected* good
case — X11 is where the risk actually lives (§10.1 lists three mitigations, and
milestone M4 will find out which of them is needed). What matters here is that
nothing in the Vello → wgpu → layer-surface path destroys the alpha channel.

Verified arithmetically rather than by eye, by `verify.py`:

| sample | measured | meaning |
|---|---|---|
| wallpaper, away from the widget | `rgb(30, 111, 217)` | untouched, exactly `#1E6FD9` |
| the glass, nothing else drawn there | `rgb(25, 43, 72)` | a blend — blue leads by 38, total light 140 vs 358 |
| a meter bar (opaque control) | `rgb(120, 190, 255)` | reads fully opaque, as it must |
| below the body | `rgb(26, 98, 191)` | the wallpaper, darkened by the shadow |

The glass reading is predictable from first principles, which is the real
check. At that point the gradient is ~61 % alpha over a ~43 % black shadow over
the background, giving `rgb(29, 48, 78)` on paper against `rgb(25, 43, 72)`
measured. If the swapchain had composited as opaque, the same pixel would read
about `rgb(21, 22, 27)` — near-neutral, with no blue in it at all.

**The opaque control earned its place.** An earlier version of the check
sampled the panel's centre, which lands on a meter bar — a solid `#78BEFF`.
That read as "very blue" and passed, and would have passed just as happily with
transparency completely broken. The control exists so that a sampling mistake
fails loudly instead of producing a false pass; it caught a second one
immediately afterwards, when the sample moved to the *third* meter, which is
deliberately red.

## §10.2 — Who drives the surface lifecycle

Resolved, and the ordering is the one the architecture assumed:

1. Create the `wl_surface`, give it the layer role, set size, anchor, margin
   and `keyboard-interactivity: none`.
2. `commit` **with no buffer** — this is what asks for the configure.
3. Receive `configure`, `ack_configure`.
4. *Only then* hand the surface to wgpu as `RawWindowHandle::Wayland`.

wgpu accepts a surface configured by somebody else without complaint. It never
needs to see the configure, and it must not be given the surface before the
handshake completes.

One thing that was not in the plan: **wgpu 29 wants the display handle at
`Instance` creation**, not only at surface creation
(`InstanceDescriptor::new_with_display_handle`). Vulkan ignores it, but the GL
backend needs it to bind EGL to this `wl_display` — and GL is the second of the
three X11 transparency mitigations, so `uniwidged-shell` must supply it from
the start rather than discovering this at M4.

## Stacking — the widget stays below ordinary windows

![The same widget, covered by an ordinary window](images/widget-under-window.png)

Proven by `stacking.py`, which screenshots the widget alone and then with an
opaque `xdg_toplevel` opened over it:

| sample | widget alone | window opened | |
|---|---|---|---|
| glass | `rgb(25, 43, 72)` | `rgb(240, 90, 30)` | covered |
| meter | `rgb(120, 190, 255)` | `rgb(240, 90, 30)` | covered |
| title | `rgb(214, 217, 222)` | `rgb(240, 90, 30)` | covered |

`Layer::Bottom` is the right choice, not `Layer::Background`: background is
where a wallpaper goes and sits *under* the desktop's own surfaces.

## The degraded path

Forcing the fallback (`SPIKE_FORCE_TOPLEVEL=1`) exercises the branch a GNOME
user takes, which would otherwise never run on a compositor that has a layer
shell:

```
surface role: Toplevel
  DEGRADED — no layer shell: the widget sits among windows rather than
  behind them, and cannot place itself — the compositor decides where it goes
```

Ephemeris's rule survives contact: say which capability is gone, in words a
user can act on, rather than degrading quietly.

## Things learned that change later milestones

* **The surface must be larger than the glass by the shadow's extent.** The
  scene's 24 px inset is not quite enough for a 14 px blur offset 6 px
  downward, and the shadow is clipped at the bottom edge. Ephemeris already
  does this — the window is larger than the visible body by `Metrics::shadow`
  on every side — and `uniwidged-compose` must own that margin rather than
  leaving it to the widget author, who cannot know the theme's shadow.
* **`zwp_linux_dmabuf_v1` was absent** under `WLR_RENDERER=pixman`, so Mesa's
  WSI fell back to `wl_shm`. It worked, which is a useful data point for the
  CPU path, but a GPU-composited session is the case M1 must measure.
* **wgpu's GL backend needs the display handle at instance creation** — see
  §10.2 above.
* **A stale instance is invisible in a screenshot.** Two overlapping layer
  surfaces produced `rgb(24, 28, 39)` where one produces `rgb(25, 43, 72)`, and
  nothing about the image said so. `run.sh` kills previous instances before
  every capture, and the numbers are only meaningful with exactly one running.

## Reproducing

```sh
./run.sh          # starts a headless sway if none is running, then both checks
```
