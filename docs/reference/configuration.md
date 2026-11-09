# Configuration files

Everything uniwidged keeps is in **one directory**, so that backing it up or
copying it to a second machine brings back your widgets *and* what they
remembered.

| | Location |
|---|---|
| Linux | `$XDG_CONFIG_HOME/uniwidged/`, else `~/.config/uniwidged/` |
| Override | `UNIWIDGED_CONFIG=/path/to/config.toml` moves the whole directory to that file's parent — keys, data, keychain, log and the running desktop's state with it |

`--config PATH`, which `run` and the commands that edit placements and
settings accept, selects **only** the configuration file; everything else stays
in the default directory. To keep a second desktop, or a test, entirely apart
from yours, use `UNIWIDGED_CONFIG`.

```text
~/.config/uniwidged/
├── config.toml          what is installed, where, granted what, set to what
├── trusted-keys         whose widget signatures this machine accepts
├── registries.toml      the registries this machine reads, and their pinned keys
├── widgets/<name>/      each widget's own data directory (fs: grants live here)
├── store/               components installed from a registry
├── registry-cache/      the last verified copy of each registry index
├── keyring.enc          the `secrets` store, encrypted
├── keyring.key          …and the key that opens it
├── uniwidged.log        what the desktop said (and .log.1, rotated)
└── uniwidged.pid/.state the running desktop, while it runs
```

All of it is plain text where it can be, and meant to be read.

::: warning A backup of this directory contains your widgets' secrets
`keyring.enc` and `keyring.key` sit side by side, so anybody who can read both
can recover every credential a widget stored. Protect a backup of this
directory like a password file, and leave both out of anything you share — a
dotfiles repository included.
:::

## `config.toml`

```toml
[[widget]]
name = "system-load"
component = "widgets/system-load.wasm"   # relative to this file, or absolute
granted = ["timer", "metrics"]
enabled = true                            # optional, default true

[widget.placement]
output = "DP-1"     # monitor name; optional
x = 48
y = 48
width = 320
height = 200
locked = false      # optional

[widget.settings]
warn-at = 0.9
```

| Key | |
|---|---|
| `name` | Must match the manifest's `name`. Also the widget's data directory and keychain namespace. |
| `component` | The `.wasm` file. |
| `manifest` | Optional. Defaults to the component's path with `.toml`. `install` writes it only when the manifest is elsewhere. |
| `granted` | What you approved. Must be a **subset** of what the manifest declares — granting more is refused at start-up. Remove an entry to decline that capability. |
| `enabled` | `false` keeps the widget installed but off the screen. |
| `placement` | Where it sits. Stored by monitor **name**, because indices renumber when a monitor is unplugged. Defaults to 320×200 at 48,48. |
| `settings` | Only values somebody set. A key at its default is absent, so a widget's change of default is not overridden by a value nobody chose. Checked against the manifest when used; a stale key does not stop the file from loading. |

After editing by hand, `uniwidged reload`. Every command that writes this file
does that for you.

## `trusted-keys`

One public key per line, optionally followed by a name. Written by
`uniwidged trust`, removed by `uniwidged untrust`, listed by
`uniwidged trusted`.

There is no key server, no certificate chain and no revocation protocol: this
file is the whole of the trust model, which is the point. A security mechanism
nobody can inspect is one nobody should rely on.

## `registries.toml`

Written by `uniwidged registry add`. Each entry is a name, the index URL, and
the public key the index must be signed with. That key says **who publishes
the list**; it is never added to `trusted-keys` and cannot vouch for a widget.
See [Finding widgets](../guide/registries.md).

## The manifest — `widget.toml`

Shipped with every widget, beside the component and covered by the same
signature. Its format is in the [API reference](./api.md#the-manifest).

## Environment variables

| Variable | Effect |
|---|---|
| `UNIWIDGED_CONFIG` | Path to `config.toml`; everything else is kept beside it. To run two desktops side by side, or test without touching your own, give each its own **directory** — two files in one directory share widget data and running state. |
| `XDG_CONFIG_HOME` | The usual XDG base directory. |
| `WGPU_BACKEND` | Force a wgpu backend (`vulkan`, `gl`) — occasionally useful on unusual drivers. |
