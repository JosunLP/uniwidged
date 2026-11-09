# Running the desktop

`uniwidged run` is the host: one process, one GPU device, one surface per
widget. Everything else on the command line either changes what it shows or
asks it what it is doing — and every command that writes the configuration
tells a running desktop at once.

## Starting it

```sh
uniwidged run                  # in the foreground, logging to the terminal
uniwidged autostart enable     # from now on, with your session
```

At start-up it says which shell it got:

| Session | Shell | What you get |
|---|---|---|
| Wayland with `wlr-layer-shell` | `Wayland (wlr-layer-shell, bottom layer)` | The real thing: a surface on the bottom layer, below every window. |
| Wayland without it (GNOME) | `Wayland (xdg_toplevel — no layer shell here)` | An ordinary window. It works, but it is not below your windows. |
| X11 | `X11` | A 32-bit ARGB window with `_NET_WM_STATE_BELOW`. Needs a compositing manager for transparency. |

If something is lost — no compositor, no alpha — the line ends in
`degraded: …` and says what. `uniwidged doctor` explains it at length.

## Installing and removing

```sh
uniwidged install path/to/widget.wasm   # signed by a key you trust
uniwidged install my.wasm --allow-unsigned   # your own build
uniwidged add clock                     # from a registry — see Finding widgets
uniwidged disable clock                 # off the screen, settings kept
uniwidged enable clock
uniwidged remove clock                  # uninstalled; its data is kept
uniwidged remove clock --purge          # …and forgotten
```

A widget appears or disappears on a running desktop without a restart, taking
its window, swapchain and outstanding work with it. An upgrade under the same
name is applied live too.

## Placing widgets

**With the pointer:** drag a widget to move it, drag its edge to resize it.
**Without one** — over SSH, from a dotfiles script, or onto a monitor that is
not plugged in right now:

```sh
uniwidged move clock --x 48 --y 48 --width 320 --height 120 --output DP-1
uniwidged lock clock      # clicks still work; dragging does not
uniwidged unlock clock
```

Placements are stored per monitor **by name** (`DP-1`, `HDMI-A-1`), never by
index, because indices renumber when a monitor is unplugged. A widget on a
monitor that goes away waits on another one, remembers where it belongs, and
goes back when the monitor returns — while the desktop is running.

## Configuring widgets

A widget declares its settings — a type, a range, a default — and the host
checks every value before the widget sees it:

```sh
uniwidged configure system-load                 # list what it offers
uniwidged configure system-load --set warn-at=0.8
uniwidged configure system-load --reset warn-at
uniwidged configure system-load --reset-all
```

A wrong value is refused here, in a sentence, rather than handed to a widget.
The widget is reconfigured in place, without a restart.

## Watching it

| Command | Answers |
|---|---|
| `uniwidged status` | What is **on the screen**: which widget is drawing, which one the desktop gave up on, where each is, which shell this session got. |
| `uniwidged list` | What the **configuration** asks for, and what each widget was granted. Marks a widget that is configured but not on screen. |
| `uniwidged log` / `log -f` | What the desktop has said — also when it was started without a terminal. One megabyte, rotated once. |
| `uniwidged doctor` | What this session can and cannot do: shell, alpha path, renderer, machine counters, registries, trusted keys. |

## Editing the configuration by hand

Everything lives in one readable file — see
[Configuration files](../reference/configuration.md). After editing it:

```sh
uniwidged reload    # SIGHUP to the running desktop
```

Commands that write the file do this for you.

## Health and restarts

Every call into a widget has a 16 ms deadline and a memory cap. A widget that
blows either loses its instance and gets a fresh one, up to three times in a
row; then the desktop gives up on it, showing its last frame, and `status`
says `given up on`. One
widget that cannot start — a deleted component, a half-edited manifest — is
named in the log, and the rest start without it.
