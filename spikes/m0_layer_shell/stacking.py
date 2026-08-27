#!/usr/bin/env python3
"""Is the widget really *below* ordinary windows?

`verify.py` proves transparency. This proves the other half of what makes a
desktop widget a desktop widget: that an ordinary application window covers it.

Take two screenshots -- the widget alone, then the same widget with an opaque
window opened over it -- and this checks that the widget's pixels became the
window's. It also checks that they were *not* the window's colour beforehand,
because a test that passes on both screenshots is testing nothing.

    python3 stacking.py before.png after.png [--x 464 --y 264 --w 360]
"""

import sys

from verify import patch, read_png

# The opaque fill `render.rs` uses under SPIKE_FORCE_TOPLEVEL.
WINDOW = (240, 90, 30)


def near(a, b, tol=30):
    return sum((x - y) ** 2 for x, y in zip(a, b)) ** 0.5 <= tol


def main():
    args = sys.argv[1:]
    if len(args) < 2:
        raise SystemExit(__doc__)
    opts = dict(zip(args[2::2], args[3::2]))
    ox = int(opts.get("--x", 464))
    oy = int(opts.get("--y", 264))
    sw = int(opts.get("--w", 360))

    _, _, before = read_png(args[0])
    _, _, after = read_png(args[1])

    # Three places the widget is unmistakably itself: translucent glass, an
    # opaque meter, and the title block.
    samples = {
        "glass": (ox + sw // 2, oy + 158),
        "meter": (ox + 60, oy + 74),
        "title": (ox + 60, oy + 50),
    }

    print(f"{'sample':>8}  {'widget alone':>22}  {'window opened':>22}")
    failures = []
    for name, (x, y) in samples.items():
        pb, pa = patch(before, x, y, size=1), patch(after, x, y, size=1)
        covered = near(pa, WINDOW)
        print(f"{name:>8}  rgb{str(pb):>19}  rgb{str(pa):>19}  "
              f"{'covered' if covered else 'STILL VISIBLE'}")
        if near(pb, WINDOW):
            failures.append(f"{name} was already the window's colour before it opened")
        if not covered:
            failures.append(f"{name} is still visible with a window over it")

    if failures:
        print("\n  FAIL")
        for f in failures:
            print(f"    - {f}")
        return 1
    print("\n  PASS — the bottom-layer surface is drawn under an ordinary window")
    return 0


if __name__ == "__main__":
    sys.exit(main())
