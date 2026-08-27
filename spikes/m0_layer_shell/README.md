# M0 · Can a desktop-layer surface back a GPU swapchain?

The whole of uniwidged rests on one claim that nobody had tested: that a
surface placed **below every window and above the wallpaper** can be rendered
by the GPU **with per-pixel alpha**. Ephemeris proved the placement half with
Cairo. Ferrum UI proved the GPU half in an ordinary window. Nobody had put the
two together, and if they do not compose, the architecture is wrong.

So this is deliberately the first thing built, before any of the framework.

## What it does

1. Connects to Wayland and dumps the registry, reporting whether
   `zwlr_layer_shell_v1` exists.
2. Creates a `wl_surface` and gives it the **bottom layer** role with
   `keyboard-interactivity: none` — or falls back to an `xdg_toplevel` and says
   what that costs, exactly as `uniwidged-shell` will.
3. Drives the `configure` / `ack_configure` handshake **itself**, then hands the
   surface to wgpu as a `RawWindowHandle::Wayland`. This ordering is the second
   open question (`ARCHITECTURE.md` §10.2): wgpu normally expects to own the
   surface lifecycle, and here it does not.
4. Prints the surface's **supported alpha modes** — the empirical answer the
   spike exists for — and configures with `PreMultiplied`.
5. Renders one Vello scene: a real Gaussian drop shadow, a translucent glass
   panel with a vertical gradient, a hairline highlight and three meters.

## What proves it

Three scripts, all reading a `grim` screenshot of the compositor output and
checking arithmetic rather than looking at pictures.

`verify.py` checks transparency. The compositor's background is a saturated blue; if the panel's
pixels are a *blend* of that blue and the panel colour, the alpha reached the
compositor. If they are the panel colour alone, it did not, and the swapchain
is opaque no matter what it reported.

`compare.py` checks that the headless render of the *same scene* composites to
the same pixels as the on-screen one. That equivalence is what makes a golden
image rendered in CI evidence about what ships — and it is what caught the
straight-versus-premultiplied alpha bug described in `FINDINGS.md`.

`stacking.py` checks that an ordinary window covers the widget.

## Running it

```sh
./run.sh          # starts a headless sway if none is running, then all three
```
