#!/usr/bin/env python3
"""Does the headless path produce the same pixels as the on-screen path?

This is the question that decides whether a golden image is worth anything. CI
has no compositor and no display, so every visual test renders through
`uniwidged-render`'s readback path. If that path and the swapchain path can
disagree, a green CI proves nothing about what a user sees.

The spike renders one scene to both in a single `render` call. This composites
the readback image over the compositor's background colour -- the arithmetic
the compositor itself performs -- and compares the result with a `grim` capture
of the real surface, pixel for pixel.

    python3 compare.py shot.png readback.png [--x 48 --y 48]
"""

import sys

from verify import BACKGROUND, read_png


def composite_over(pixel, background):
    """Source-over with a straight-alpha source, which is what a PNG holds."""
    r, g, b, a = pixel
    f = a / 255.0
    return tuple(round(c * f + bg * (1.0 - f)) for c, bg in zip((r, g, b), background))


def main():
    args = sys.argv[1:]
    if len(args) < 2:
        raise SystemExit(__doc__)
    opts = dict(zip(args[2::2], args[3::2]))
    ox = int(opts.get("--x", 48))
    oy = int(opts.get("--y", 48))

    sw, sh, shot = read_png(args[0])
    rw, rh, back = read_png(args[1])
    print(f"screenshot: {sw}x{sh}   readback: {rw}x{rh}")
    print(f"comparing the surface at {ox},{oy}\n")

    if ox + rw > sw or oy + rh > sh:
        raise SystemExit("the readback image does not fit in the screenshot at that offset")

    worst, worst_at, total, count, opaque_seen = 0, None, 0, 0, False
    for y in range(rh):
        for x in range(rw):
            src = back[y][x]
            if src[3] == 255:
                opaque_seen = True
            expected = composite_over(src, BACKGROUND)
            actual = shot[oy + y][ox + x][:3]
            d = max(abs(e - a) for e, a in zip(expected, actual))
            total += d
            count += 1
            if d > worst:
                worst, worst_at = d, (x, y, expected, actual)

    mean = total / count
    print(f"  pixels compared: {count}")
    print(f"  mean channel difference: {mean:.3f}")
    print(f"  worst channel difference: {worst}")
    if worst_at:
        x, y, e, a = worst_at
        print(f"    at ({x},{y}): headless→{e}  on-screen→{a}")

    failures = []
    # A wholly transparent readback image would make every comparison trivially
    # succeed, so check that something was actually drawn.
    if not opaque_seen:
        failures.append("the readback image has no opaque pixel — nothing was drawn")
    # One unit of rounding is expected: the compositor blends in 8-bit and so
    # does this script, but not necessarily in the same order.
    if worst > 2:
        failures.append(f"the two paths disagree by up to {worst} per channel")
    if mean > 0.5:
        failures.append(f"the two paths differ on average by {mean:.3f} per channel")

    if failures:
        print("\n  FAIL")
        for f in failures:
            print(f"    - {f}")
        return 1
    print("\n  PASS — the headless path and the on-screen path agree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
