#!/usr/bin/env bash
# The whole spike, end to end, against a nested headless compositor.
# Needs: sway, grim, a Vulkan driver (lavapipe is enough), python3.
set -euo pipefail

cd "$(dirname "$0")"
OUT=${OUT:-/tmp/uniwidged-m0}
mkdir -p "$OUT"
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}

if [ -z "${WAYLAND_DISPLAY:-}" ]; then
    echo "starting a headless sway..."
    WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
        sway -c sway.conf >"$OUT/sway.log" 2>&1 &
    for _ in $(seq 1 20); do
        s=$(ls "$XDG_RUNTIME_DIR" 2>/dev/null | grep -E '^wayland-[0-9]+$' | head -1) && [ -n "$s" ] && break
        sleep 0.5
    done
    export WAYLAND_DISPLAY=$s
fi
echo "compositor: $WAYLAND_DISPLAY"

cargo build --release
BIN=./target/release/m0-layer-shell
export SPIKE_HOLD_SECS=${SPIKE_HOLD_SECS:-45}

up() { for _ in $(seq 1 20); do grep -q 'frame presented' "$1" 2>/dev/null && return; sleep 1; done; }

# 1. Transparency, and the headless path against the on-screen one.
pkill -x m0-layer-shell 2>/dev/null || true; sleep 1
SPIKE_READBACK_PNG="$OUT/readback.png" setsid $BIN >"$OUT/widget.log" 2>/dev/null &
up "$OUT/widget.log"
cat "$OUT/widget.log"
grim -o HEADLESS-1 "$OUT/widget.png"
python3 verify.py "$OUT/widget.png"
echo
python3 compare.py "$OUT/widget.png" "$OUT/readback.png"

# 2. Stacking: the same widget, moved under where the window will open.
pkill -x m0-layer-shell 2>/dev/null || true; sleep 1
SPIKE_MARGIN_TOP=264 SPIKE_MARGIN_LEFT=464 setsid $BIN >"$OUT/under.log" 2>/dev/null &
up "$OUT/under.log"
grim -o HEADLESS-1 "$OUT/before.png"
SPIKE_FORCE_TOPLEVEL=1 setsid $BIN >"$OUT/window.log" 2>/dev/null & up "$OUT/window.log"
sleep 2
grim -o HEADLESS-1 "$OUT/after.png"
python3 stacking.py "$OUT/before.png" "$OUT/after.png"

pkill -x m0-layer-shell 2>/dev/null || true
echo; echo "screenshots in $OUT"
