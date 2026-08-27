#!/usr/bin/env python3
"""Does the wallpaper show through the widget on X11?

X11 is where `ARCHITECTURE.md` §10.1 says the risk lives, and it has a failure
mode Wayland does not: **the surface can report an alpha mode it does not
deliver.** Asking wgpu is not evidence here; only the pixels are.

So this does not check for "some blue". It renders the same scene twice — once
to the window, once through readback — and composites the headless image over
two candidate backdrops:

    over the wallpaper   what a correctly composited widget looks like
    over black           what a widget whose alpha was dropped looks like

Whichever matches is what happened. Reporting both is the point: a single
number cannot tell "transparent" from "opaque over a dark scene", and the glass
is dark enough that eyeballing it cannot either.

No third-party imports; the PNG reader is shared with the M0 spike.

    python3 verify.py shot.png readback.png [--x 48 --y 48]
"""

import sys
from pathlib import Path

# One decoder, shared with the Wayland spike rather than copied.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "m0_layer_shell"))
from verify import BACKGROUND, patch, read_png  # noqa: E402

BLACK = (0, 0, 0)


def composite_over(pixel, backdrop):
    """Source-over with a straight-alpha source, which is what a PNG holds."""
    r, g, b, a = pixel
    f = a / 255.0
    return tuple(round(c * f + d * (1.0 - f)) for c, d in zip((r, g, b), backdrop))


def agreement(shot, back, origin, backdrop):
    """Mean and worst channel difference between the screenshot and the
    headless render composited over `backdrop`."""
    ox, oy = origin
    width, height, rows = back
    total = worst = 0
    for y in range(height):
        for x in range(width):
            expected = composite_over(rows[y][x], backdrop)
            actual = shot[2][oy + y][ox + x][:3]
            delta = max(abs(e - a) for e, a in zip(expected, actual))
            total += delta
            worst = max(worst, delta)
    return total / (width * height), worst


def main():
    args = sys.argv[1:]
    if len(args) < 2:
        raise SystemExit(__doc__)
    opts = dict(zip(args[2::2], args[3::2]))
    origin = (int(opts.get("--x", 48)), int(opts.get("--y", 48)))

    shot = read_png(args[0])
    back = read_png(args[1])
    print(f"screenshot: {shot[0]}x{shot[1]}   readback: {back[0]}x{back[1]}")
    print(f"widget at:  {origin[0]},{origin[1]}\n")

    away = patch(shot[2], shot[0] - 30, shot[1] - 30)
    print(f"  wallpaper, far from the widget: rgb{away}")

    on_wallpaper = agreement(shot, back, origin, BACKGROUND)
    on_black = agreement(shot, back, origin, BLACK)
    print(f"  the same scene over the wallpaper: mean {on_wallpaper[0]:6.2f}  worst {on_wallpaper[1]:3d}")
    print(f"  the same scene over black:         mean {on_black[0]:6.2f}  worst {on_black[1]:3d}")

    failures = []
    if max(abs(a - b) for a, b in zip(away, BACKGROUND)) > 6:
        failures.append(f"the wallpaper away from the widget is rgb{away}, not rgb{BACKGROUND}")

    # A widget whose alpha never reached the compositor matches "over black"
    # closely and "over the wallpaper" not at all. The comparison is between
    # the two rather than against a threshold, because the glass is dark and
    # both numbers are small in absolute terms.
    if on_wallpaper[0] > on_black[0]:
        failures.append(
            "the window matches 'over black' better than 'over the wallpaper' — "
            "the alpha channel did not reach the compositor, whatever the "
            "surface reported"
        )
    elif on_wallpaper[0] > 3.0:
        failures.append(
            f"the window matches neither backdrop well (mean {on_wallpaper[0]:.2f}); "
            "something else is compositing into it — a compositor's own drop "
            "shadow will do this"
        )
    else:
        print(
            f"\n  [ok] the window is the scene composited over the wallpaper, "
            f"to within {on_wallpaper[1]} per channel"
        )

    if failures:
        print("\n  FAIL")
        for f in failures:
            print(f"    - {f}")
        return 1
    print("\n  PASS — per-pixel alpha reached the compositor on X11")
    return 0


if __name__ == "__main__":
    sys.exit(main())
