# Troubleshooting

Start with the two commands that were written for this page:

```sh
uniwidged doctor    # what this session can and cannot do
uniwidged log -n 50 # what the desktop has been saying
```

## The widget has an opaque background

The start-up line ends in `degraded: no per-pixel alpha; the panel gets an
opaque backing`. There is no compositing manager, so there is no per-pixel
alpha.

* **X11:** start one — `picom`, or your desktop's built-in compositor.
  `doctor` names the alpha path it found.
* **Wayland:** every Wayland compositor composites; if you see this, `doctor`
  will say what the swapchain offered.

## The widget floats above my windows

You are on a Wayland compositor without `wlr-layer-shell` — GNOME is the
common case — and the host fell back to an ordinary window. The start-up line
says `xdg_toplevel — no layer shell here`. Use a compositor with layer-shell
support, or an X11 session.

## A widget is installed but not on the screen

```sh
uniwidged list      # "(not running)" or "(given up on)" beside its name
uniwidged status
uniwidged log | grep <name>
```

Common reasons, in the words the log uses:

| Log says | Meaning | Fix |
|---|---|---|
| `built for ABI 8; this host runs ABI 9 to 9` | Built against a pre-1.0 SDK. | Rebuild it against the current SDK. |
| `newer than this host's 9; upgrading uniwidged will run it` | Built for a newer 1.x. | Upgrade uniwidged. |
| `the widget stopped: …` | It trapped or overran its 16 ms deadline, three times in a row. | A bug in the widget. The desktop has given up on it and shows its last frame. |
| `is granted …, which its manifest never asks for` | `config.toml` grants more than the manifest declares. | Remove the extra entry from `granted`. |
| `the widget refused to start: …` | The widget's own `start` said no. | Whatever it says. |

## A meter shows a dash

The capability was granted but this machine cannot answer it — a desktop has
no battery, a container has no network counters. `doctor` lists what every
machine counter answers here. A widget that shows a dash rather than a wrong
number is doing the right thing.

## `install` says the widget is not signed

That is the default for a widget you did not write. Either:

* you built it yourself — `uniwidged install … --allow-unsigned`, or
* somebody sent it to you — ask for their public key, check it with them, then
  `uniwidged trust <key> --name "who they are"`.

A widget whose signature **does not verify** is never installable, flag or
not: a bad signature is a positive claim that something is wrong.

## `install` cannot find the manifest

`install` looks for `<component>.toml` beside the `.wasm`, then for the
`widget.toml` of the crate it was built in. If you build into a custom
`CARGO_TARGET_DIR`, point at it:

```sh
uniwidged install path/to/widget.wasm --manifest path/to/widget.toml
```

## `update` will not take a new version

`update` is the one command that installs something nobody looked at, so it
refuses a version that asks for **more** than the installed one was granted.
It says which capability is new; `uniwidged add <name>` shows the whole list
and asks.

## Two desktops, or a stale one

`uniwidged status` checks that the recorded process is alive **and** is
uniwidged before believing it. If a desktop was killed outright, the next
`run` takes over cleanly.

## Still stuck?

Open an issue with the output of `uniwidged --version`, `uniwidged doctor` and
the relevant part of `uniwidged log` (with tokens, credentials and private URLs
removed — issues are public) —
[github.com/JosunLP/uniwidged/issues](https://github.com/JosunLP/uniwidged/issues).
