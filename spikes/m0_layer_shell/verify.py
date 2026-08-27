#!/usr/bin/env python3
"""Does the wallpaper actually show through the widget?

The spike prints what the surface *reported*. This checks what the compositor
actually composited, which is a different question: a swapchain can advertise
`PreMultiplied` and still be blended as opaque further down.

The proof is arithmetic. The compositor's background is a saturated blue and
the glass is a near-neutral dark grey at ~60 % alpha, so a correctly composited
pixel is a predictable blend of the two -- and a wrong one is not:

    glass over wallpaper   ~ rgb(33, 66, 114)   blue leads, but darkened
    glass composited opaque  ~ rgb(21, 22,  27)   neutral grey, no blue at all
    nothing drawn            = rgb(30, 111, 217)   the wallpaper itself

Sampling is the part that has to be careful. The scene contains opaque elements
-- the meter bars are solid #78BEFF -- and a sample that lands on one reads as
"very blue" whether or not any alpha survived. So the glass is sampled at a
point chosen to contain nothing else, and a meter is sampled separately as a
control: it must read opaque, or the test cannot tell the two cases apart.

No third-party imports on purpose: a proof nobody can run is not a proof.

    python3 verify.py shot.png [--x 48 --y 48 --w 360 --h 200]
"""

import struct
import sys
import zlib

# Must match sway.conf. Change one, change the other.
BACKGROUND = (0x1E, 0x6F, 0xD9)
# The opaque meter colour from `render.rs::widget_scene`.
METER = (0x78, 0xBE, 0xFF)


def read_png(path):
    """Returns (width, height, rows), rows being lists of (r, g, b)."""
    with open(path, "rb") as f:
        data = f.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(f"{path} is not a PNG")

    pos, idat, width, height, depth, colour = 8, bytearray(), 0, 0, 0, 0
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            width, height, depth, colour = struct.unpack(">IIBB", body[:10])
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break
        pos += 12 + length

    if depth != 8 or colour not in (2, 6):
        raise SystemExit(f"expected 8-bit RGB or RGBA, got depth={depth} colour={colour}")
    channels = 3 if colour == 2 else 4

    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    out, prev = [], bytearray(stride)
    pos = 0
    for _ in range(height):
        filter_type = raw[pos]
        line = bytearray(raw[pos + 1 : pos + 1 + stride])
        pos += 1 + stride
        for i in range(stride):  # the five PNG filters, undone in place
            a = line[i - channels] if i >= channels else 0
            b = prev[i]
            c = prev[i - channels] if i >= channels else 0
            if filter_type == 1:
                line[i] = (line[i] + a) & 0xFF
            elif filter_type == 2:
                line[i] = (line[i] + b) & 0xFF
            elif filter_type == 3:
                line[i] = (line[i] + (a + b) // 2) & 0xFF
            elif filter_type == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 0xFF
        out.append([tuple(line[i : i + 3]) for i in range(0, stride, channels)])
        prev = line
    return width, height, out


def distance(a, b):
    return sum((x - y) ** 2 for x, y in zip(a, b)) ** 0.5


def patch(rows, x, y, size=3):
    """The mean of a small patch, so one stray anti-aliased pixel cannot decide
    the outcome."""
    acc, n = [0, 0, 0], 0
    for dy in range(-size, size + 1):
        for dx in range(-size, size + 1):
            px = rows[y + dy][x + dx]
            acc = [a + b for a, b in zip(acc, px)]
            n += 1
    return tuple(round(c / n) for c in acc)


def main():
    args = sys.argv[1:]
    if not args:
        raise SystemExit(__doc__)
    path = args[0]
    opts = dict(zip(args[1::2], args[2::2]))
    ox = int(opts.get("--x", 48))   # the surface's origin, from set_margin
    oy = int(opts.get("--y", 48))
    sw = int(opts.get("--w", 360))
    sh = int(opts.get("--h", 200))

    width, height, rows = read_png(path)
    print(f"screenshot: {width}x{height}")
    print(f"surface at: {ox},{oy} {sw}x{sh}\n")

    # Surface-local coordinates, matching `render.rs::widget_scene`:
    #   body inset 24, title bar y 46..56, meters at y 70/100/130 (8 tall).
    # Below the last meter and above the body's bottom edge there is nothing
    # but glass -- that is the only place worth sampling.
    glass = patch(rows, ox + sw // 2, oy + 158)
    # The *first* meter, at surface y 70..78. Not the third: that one is
    # deliberately red (its value crosses the critical threshold), which is
    # exactly the sort of thing this control exists to catch.
    meter = patch(rows, ox + 60, oy + 74, size=1)
    outside = patch(rows, width - 30, height - 30)
    # Inside the surface, below the body, where the blurred shadow falls.
    shadow = patch(rows, ox + sw // 2, oy + 188, size=2)

    print(f"  wallpaper, far from the widget : rgb{outside}")
    print(f"  glass, nothing else drawn there: rgb{glass}")
    print(f"  meter bar (opaque, control)    : rgb{meter}")
    print(f"  below the body (shadow)        : rgb{shadow}\n")

    failures = []

    if distance(outside, BACKGROUND) > 12:
        failures.append(f"the wallpaper away from the widget is wrong: rgb{outside}")
    else:
        print("  [ok] away from the widget the wallpaper is untouched")

    # The control. If the opaque meter does not read as its own colour, the
    # sampling coordinates are wrong and nothing below can be trusted.
    if distance(meter, METER) > 40:
        failures.append(
            f"the opaque meter reads rgb{meter}, expected ~rgb{METER} — the "
            "sample coordinates are off, so no other result means anything"
        )
    else:
        print("  [ok] the opaque control element reads as fully opaque")

    if distance(glass, BACKGROUND) < 25:
        failures.append("the glass is indistinguishable from the wallpaper — nothing was drawn")
    elif glass[2] <= max(glass[0], glass[1]) + 10:
        # The glass's own colour is near-neutral (r~=g~=b~=25). Blue can only
        # lead if the wallpaper bled through it.
        failures.append(
            f"the glass reads rgb{glass}: near-neutral, so no wallpaper came "
            "through — the swapchain composited as OPAQUE"
        )
    elif sum(glass) >= sum(BACKGROUND):
        failures.append(f"the glass rgb{glass} is not darker than the wallpaper")
    else:
        bleed = glass[2] - (glass[0] + glass[1]) / 2
        print(f"  [ok] the wallpaper shows through the glass, darkened "
              f"(blue leads by {bleed:.0f}, total light {sum(glass)} < {sum(BACKGROUND)})")

    # A real Gaussian shadow darkens the wallpaper without replacing it.
    if distance(shadow, BACKGROUND) > 8 and sum(shadow) < sum(BACKGROUND):
        print(f"  [ok] the drop shadow darkens the wallpaper rather than covering it")
    else:
        print("  [--] no measurable shadow (it may be clipped by the surface bounds)")

    if failures:
        print("\n  FAIL")
        for f in failures:
            print(f"    - {f}")
        return 1
    print("\n  PASS — per-pixel alpha reached the compositor")
    return 0


if __name__ == "__main__":
    sys.exit(main())
