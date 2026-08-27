#!/usr/bin/env python3
"""Is the widget really *below* ordinary windows on X11?

`_NET_WM_STATE_BELOW` is a request to the window manager, not a fact. This
takes two screenshots — the widget alone, then an ordinary opaque window opened
over the same place — and checks that the widget's pixels became the window's.

It also checks they were *not* the window's beforehand, because a test that
passes on both screenshots is testing nothing.

    python3 stacking.py before.png after.png [--x 48 --y 48]
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "m0_layer_shell"))
from verify import patch, read_png  # noqa: E402


def main():
    args = sys.argv[1:]
    if len(args) < 2:
        raise SystemExit(__doc__)
    opts = dict(zip(args[2::2], args[3::2]))
    ox = int(opts.get("--x", 48))
    oy = int(opts.get("--y", 48))

    _, _, before = read_png(args[0])
    _, _, after = read_png(args[1])

    # Three points inside the widget, chosen to be different from each other so
    # that "they all became one colour" is a meaningful observation.
    samples = {"title row": (ox + 40, oy + 30), "a meter": (ox + 60, oy + 100), "glass": (ox + 160, oy + 170)}

    print(f"{'sample':>10}  {'widget alone':>22}  {'window opened':>22}")
    readings = []
    for name, (x, y) in samples.items():
        a, b = patch(before, x, y, size=2), patch(after, x, y, size=2)
        readings.append((name, a, b))
        print(f"{name:>10}  rgb{str(a):>19}  rgb{str(b):>19}")

    # The opaque fill the spike draws for an ordinary window.
    window = (240, 90, 30)

    def near(a, b, tol=12):
        return max(abs(p - q) for p, q in zip(a, b)) <= tol

    failures = []
    for name, alone, opened in readings:
        if not near(opened, window):
            failures.append(f"{name} still shows rgb{opened} with a window over it")
        if near(alone, window):
            failures.append(f"{name} was already the window's colour before it opened")

    if failures:
        print("\n  FAIL")
        for f in failures:
            print(f"    - {f}")
        return 1
    print("\n  PASS — the widget is drawn under an ordinary window")
    return 0


if __name__ == "__main__":
    sys.exit(main())
