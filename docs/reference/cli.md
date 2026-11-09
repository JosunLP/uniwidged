# Command line

One binary, `uniwidged`, is both the desktop and everything around it. Every
command that writes the configuration also tells a running desktop, so changes
appear without a restart.

```sh
uniwidged help              # every command, one line each
uniwidged help <command>    # usage and flags for one
uniwidged <command> --help  # the same
man uniwidged               # the manual page, installed by the packages
```

`UNIWIDGED_CONFIG=/path/to/config.toml` points every command — and everything
kept beside the configuration — at another directory. `--config PATH`, accepted
by `run` and the commands that edit placements and settings, selects only the
file. See [Configuration files](./configuration.md).

## The desktop

### run

```sh
uniwidged run [--config PATH]
```

Starts the desktop in the foreground. Picks Wayland or X11 at start-up and
says which; logs to the terminal **and** to `uniwidged.log` beside the
configuration. One widget that cannot start is named and skipped; the rest
start. `SIGHUP` makes it read its configuration again.

### status

```sh
uniwidged status
```

What a running desktop says it is doing: its pid and start time, the shell it
got, and for every widget its version, its health (`running`, `restarting` or
`given up on`), its size and position, its monitor and whether it is locked.
The recorded pid is checked to be alive and to be uniwidged before it is
believed.

### log

```sh
uniwidged log [-n N | --lines N] [-f | --follow]
```

The last lines of what the desktop has said, with or without a terminal. The
log is capped at one megabyte and rotated once to `uniwidged.log.1`.

### reload

```sh
uniwidged reload
```

Sends `SIGHUP` to the running desktop, which reads its configuration again and
applies the difference — widgets added, removed, moved or reconfigured — live.
Only needed after editing `config.toml` by hand.

### doctor

```sh
uniwidged doctor
```

What this session can and cannot do: the window system and shell, the alpha
path, the renderer and its backend, what every machine counter (CPU, memory,
disk, battery, network) answers here, the registries this machine reads and
how many keys it trusts.

### autostart

```sh
uniwidged autostart [enable | disable | status]
```

Manages an XDG autostart entry so the desktop starts with your session.
Refuses to overwrite an entry it did not write.

## Widgets on this machine

### install

```sh
uniwidged install <widget.wasm> [--manifest FILE] [--allow-unsigned]
```

Checks the signature, prints what the widget asks for, and records it with
everything it declared granted. The manifest is, in order: `--manifest`;
`<widget>.toml` beside the component; the `widget.toml` of the Cargo crate the
component was built in. Unsigned widgets need `--allow-unsigned`; a widget
whose signature **fails** is never installed. Installing a new version of an
installed widget keeps its placement, settings and enabled state.

### check

```sh
uniwidged check <widget.wasm> [--manifest FILE]
```

Everything `install` would say — name, version, signature, the capability list
— without installing anything. Also instantiates the component in a deny-all
sandbox to prove it is a widget this host can run.

### list

```sh
uniwidged list [--names]
```

What the configuration asks for: each widget, its placement and what it was
granted. With a desktop running, a widget that is configured but not on the
screen is marked. `--names` prints one name per line, for scripts.

### enable / disable

```sh
uniwidged enable <name>
uniwidged disable <name>
```

Turns a widget on or off without forgetting its placement, settings or data.

### remove

```sh
uniwidged remove <name> [--purge]
```

Uninstalls a widget. Its data directory and keychain entries are kept unless
`--purge` is given.

### configure

```sh
uniwidged configure <name>                 # what it offers, and current values
uniwidged configure <name> --set key=value [--set key=value …]
uniwidged configure <name> --reset key
uniwidged configure <name> --reset-all
```

Changes a widget's declared settings. Every value is checked against the
manifest's declaration — type, range, options — and refused with a sentence if
it does not fit. The widget is told with `Event::Configured`, without a
restart.

### move

```sh
uniwidged move <name> [--x N] [--y N] [--width N] [--height N] [--output NAME]
```

Places a widget without a pointer. Only the flags given change. `--output` is
a monitor name as `doctor` and `xrandr` report it.

### lock / unlock

```sh
uniwidged lock <name>
uniwidged unlock <name>
```

A locked widget still updates and still takes clicks; it just cannot be
dragged or resized.

### snapshot

```sh
uniwidged snapshot <name> [-o FILE]
```

Renders one installed widget headlessly to a PNG, through the same pipeline
the desktop uses — useful for a README, a bug report or a registry listing.

## Registries

See [Finding widgets](../guide/registries.md) for the model behind these.

### registry

```sh
uniwidged registry add <url> --key <public-key> [--name N]
uniwidged registry remove <name>
uniwidged registry list
uniwidged registry publish --key FILE --base-url URL [--out FILE] <widget.wasm>…
```

`add` pins the key the index is signed with. `publish` writes and signs an
index for the given widgets, to be uploaded beside them.

### search

```sh
uniwidged search [term] [--refresh] [--offline]
```

What the registries offer, with each entry's capabilities. Uses the cache
unless it is stale or `--refresh` is given; `--offline` never fetches.

### add

```sh
uniwidged add <name>[@version] [--registry NAME]
```

Downloads a widget, checks the index entry against the manifest that arrived
and the signature against **your** trusted keys, then installs it like
`install`.

### update

```sh
uniwidged update [name] [--check]
```

Installs newer versions of widgets that came from a registry. Refuses a
version that asks for more than the installed one was granted, and sends you to
`add` for that one. `--check` only reports.

## Signing and trust

### keygen

```sh
uniwidged keygen [--out FILE]
```

Generates an Ed25519 signing key and prints its public half.

### sign

```sh
uniwidged sign <widget.wasm> --key FILE [--manifest FILE]
```

Signs the component **and** its manifest together, writing `<widget>.sig`
beside the component.

### trust / untrust / trusted

```sh
uniwidged trust <public-key> [--name "who"]
uniwidged untrust <public-key>
uniwidged trusted
```

Edits and shows `trusted-keys`, the whole of this machine's trust model.

## Authoring

### new

```sh
uniwidged new <name> [--dir DIR] [--sdk PATH] [--capability CAP]…
```

Scaffolds a widget crate that already builds, draws, responds to a click,
declares a setting and has tests. Inside a checkout of this repository it uses
the SDK by path; elsewhere by git at the tag of the running binary; `--sdk`
overrides both.

### completions

```sh
uniwidged completions <bash | zsh | fish>
```

A completion script, written by the binary from the commands it actually has.
Widget names complete from what is installed.

## Exit status

`0` on success, `1` on any failure, with the reason on standard error after
`uniwidged:`.
