# M0 / M1 — Findings

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
| the glass, nothing else drawn there | `rgb(21, 39, 67)` | a blend — blue leads by 37, total light 127 vs 358 |
| a meter bar (opaque control) | `rgb(120, 190, 255)` | reads fully opaque, as it must |
| below the body | `rgb(26, 98, 191)` | the wallpaper, darkened by the shadow |

If the swapchain had composited as opaque, the same pixel would read about
`rgb(21, 22, 27)` — near-neutral, with no blue in it at all.

*(These numbers are from after the M1 alpha fix below. The M0 run measured
`rgb(25, 43, 72)` here, a little too bright, and nothing at the time was in a
position to notice.)*

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
| glass | `rgb(21, 39, 67)` | `rgb(240, 90, 30)` | covered |
| meter | `rgb(120, 190, 255)` | `rgb(240, 90, 30)` | covered |
| title | `rgb(206, 209, 214)` | `rgb(240, 90, 30)` | covered |

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

---

# M1 — what driving the real renderer turned up

Since milestone M1 this spike no longer has a renderer of its own: the device,
the surfaces and the presentation live in `uniwidged-render`, and this program
supplies a Wayland surface and a scene. That makes it a standing integration
test of the crate against a real compositor, and it immediately earned its
keep.

## The frame, measured on a compositor

```
frame: 2 rendered, 0 skipped, 3 submits
a second frame with nothing changed: 0 rendered, 0 submits
```

Two surfaces — the layer surface and a readback surface — rasterised in one
`render` call. Three submits: one from Vello per surface, plus one for both
presentations together. The second frame changed nothing and cost nothing,
which is the claim the whole cost model rests on, asserted where a real
compositor could contradict it.

## The alpha bug the cross-path comparison found

The spike now renders **the same scene twice**, once to the layer surface and
once through readback, so `compare.py` can composite the headless image over
the compositor's background and compare it with a `grim` capture of the real
one. The first run of that comparison:

```
  mean channel difference: 4.852
  worst channel difference: 110
    at (42,23): headless→(48, 115, 203)  on-screen→(158, 225, 255)
```

The cause is a mismatch that neither API declares:

* **Vello writes straight (separated) alpha.** `vello_shaders`' `fine.wgsl`
  divides the accumulated colour by its alpha before `textureStore`.
* **Wayland wants premultiplied.** `PreMultiplied` was the only non-opaque mode
  the surface offered, and `wgpu::util::TextureBlitter` is a plain copy.

So the compositor was multiplying already-separated colours as if they were
premultiplied, and every translucent pixel came out too bright — subtly enough
that the glass still looked like glass, which is exactly why it survived M0's
screenshot and M0's own `verify.py`. A check that only ever looks at one path
cannot see this.

`uniwidged-render` now does the conversion in its own blit shader, chosen from
the surface's `CompositeAlphaMode`. After the fix:

```
  mean channel difference: 0.272
  worst channel difference: 1
```

One unit, which is 8-bit rounding. **The headless path and the on-screen path
agree**, and that is what makes a golden image rendered in CI evidence about
what a user sees rather than a picture of a second, unrelated renderer.

A wrong guess along the way is worth recording too: the first attempt assumed
Vello produced premultiplied output and "fixed" `write_png` to divide it back
out. A unit test asserting the premultiplication contradicted it within a
minute, and reading `fine.wgsl` settled it. The conversion belongs in the blit,
not in the PNG writer.

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
  surfaces produced `rgb(24, 28, 39)` where one produces `rgb(21, 39, 67)`, and
  nothing about the image said so. `run.sh` kills previous instances before
  every capture, and the numbers are only meaningful with exactly one running.
* **Alpha convention is not something to assume.** Neither Vello nor wgpu
  states, in a signature, whether the pixels crossing between them are
  premultiplied. The only way this was ever going to be caught was by rendering
  the same scene down two paths and comparing — which is now what `compare.py`
  does on every run.

## Reproducing

```sh
./run.sh          # starts a headless sway if none is running, then all three checks
```

* `verify.py` — the wallpaper shows through, measured against the blend it
  should be.
* `compare.py` — the headless path and the on-screen path agree.
* `stacking.py` — an ordinary window covers the widget.
