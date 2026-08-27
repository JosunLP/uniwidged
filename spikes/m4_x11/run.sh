#!/usr/bin/env bash
# The whole M4 spike, end to end, against a nested X11 session.
# Needs: Xvfb, openbox, xcompmgr, hsetroot, ImageMagick, a Vulkan driver.
set -euo pipefail

cd "$(dirname "$0")"
OUT=${OUT:-/tmp/uniwidged-m4}
mkdir -p "$OUT"
export DISPLAY=${DISPLAY:-:99}

started_session=0
if ! xdpyinfo >/dev/null 2>&1; then
    echo "starting a nested X11 session on $DISPLAY..."
    Xvfb "$DISPLAY" -screen 0 1280x720x24 +extension Composite +extension RENDER \
        >"$OUT/xvfb.log" 2>&1 &
    for _ in $(seq 1 20); do xdpyinfo >/dev/null 2>&1 && break; sleep 0.5; done
    openbox >"$OUT/openbox.log" 2>&1 &
    sleep 1
    started_session=1
fi

# The wallpaper has to be a *root pixmap*, not just a root colour: a compositor
# composites `_XROOTPMAP_ID`, and `xsetroot -solid` does not set it. Without
# this the compositor paints grey and there is nothing to be transparent
# against.
hsetroot -solid '#1E6FD9'

# Restarted after the wallpaper, because a compositor reads the root pixmap
# once at start-up. And deliberately *without* `-c`: xcompmgr's own drop
# shadows would darken the wallpaper behind the widget, and then the arithmetic
# below would be comparing against a backdrop it does not know.
pkill -x xcompmgr 2>/dev/null || true
sleep 1
xcompmgr >"$OUT/xcompmgr.log" 2>&1 &
sleep 2

cargo build --release
BIN=./target/release/m4-x11
export SPIKE_HOLD_SECS=${SPIKE_HOLD_SECS:-40}

up() { for _ in $(seq 1 25); do grep -q "$2" "$1" 2>/dev/null && return; sleep 1; done; }
shot() { sleep 2; import -window root -depth 8 -type TrueColor "png24:$1"; }

# 1. Transparency, and the headless path against the on-screen one.
pkill -x m4-x11 2>/dev/null || true; sleep 1
SPIKE_READBACK_PNG="$OUT/readback.png" setsid $BIN >"$OUT/widget.log" 2>/dev/null &
up "$OUT/widget.log" headlessly
cat "$OUT/widget.log"
shot "$OUT/widget.png"
python3 verify.py "$OUT/widget.png" "$OUT/readback.png"

# 2. Stacking: an ordinary opaque window over the same place.
SPIKE_ORDINARY_WINDOW=1 SPIKE_X=48 SPIKE_Y=48 setsid $BIN >"$OUT/window.log" 2>/dev/null &
up "$OUT/window.log" presented
shot "$OUT/after.png"
python3 stacking.py "$OUT/widget.png" "$OUT/after.png"

pkill -x m4-x11 2>/dev/null || true
[ "$started_session" = 1 ] && echo "(the nested session is still running on $DISPLAY)"
echo; echo "screenshots in $OUT"
