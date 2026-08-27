# M4 · X11 — transparent, and below windows?

`ARCHITECTURE.md` §10.1 names X11 as the project's highest technical risk.
Ephemeris got a transparent window by drawing into a 32-bit visual with Cairo,
and Cairo does not care what the GPU thinks. A Vulkan swapchain does.

This spike creates a real desktop-layer window through `uniwidged-shell`, hands
it to `uniwidged-render`, and draws a real widget composed by
`uniwidged-compose` — so what is on screen is what a widget actually looks like,
not a hand-drawn stand-in.

## What proves it

X11 has a failure mode Wayland does not: **the surface can report an alpha mode
it does not deliver.** Asking wgpu is not evidence; only the pixels are.

So `verify.py` renders the same scene twice — once to the window, once through
readback — and composites the headless image over two candidate backdrops:
the wallpaper, and black. Whichever matches is what happened. Reporting both is
the point: one number cannot tell "transparent" from "opaque over a dark scene".

`stacking.py` checks that an ordinary opaque window covers the widget, because
`_NET_WM_STATE_BELOW` is a request to the window manager rather than a fact.

## Running it

```sh
./run.sh                        # starts a nested X11 session if none is running
SPIKE_FORCE_READBACK=1 ./run.sh # the same, taking the readback fallback
```

Findings, including three ways to set the session up wrongly, are in
[`FINDINGS.md`](FINDINGS.md).
