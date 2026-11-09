# Capabilities

A widget can reach **nothing** it did not declare. Its manifest lists what it
wants, the user sees that list when installing it, and the host enforces
exactly what was granted — the declared list, minus anything the user took
away.

```toml
# widget.toml
name = "weather"
version = "1.0.0"
description = "Today's forecast for one place"
capabilities = ["timer", "net:api.open-meteo.com", "fs:cache", "link:open-meteo.com"]
memory-mb = 16
```

<CapabilityGrid />

## Why capabilities are sentences

`net:api.open-meteo.com`, not `network`. "This widget talks to
api.open-meteo.com" is something a person can agree or object to; "this widget
uses the network" is not. The same goes for `fs:cache` — a grant for `cache`,
not for its siblings and not for the directory above it — and
`command:uptime`, which is that one command and nothing joined to a path.

`link:<host>` is a separate line from `net:<host>` on purpose: "talks to a
server" and "can send you to a web page" are different things happening to a
user.

## What install shows

```text
  weather 1.0.0
  signed by Ada Example
  Today's forecast for one place

  This widget wants to:
    · update itself on a schedule
    · talk to api.open-meteo.com over HTTPS
    · read and write `cache` in its own folder
    · open open-meteo.com in your browser when you click a link
  It may use up to 16 MB of memory.
```

To decline one, remove it from `granted` in the
[configuration file](../reference/configuration.md). The widget keeps running
and is answered `NotGranted` for that capability.

## Three kinds of no

Every host call returns a `Result`, and the refusals are distinguished,
because each one is fixed by somebody different:

| Refusal | Means | Fixed by |
|---|---|---|
| `Denied::NotGranted` | The manifest never asked, or the user declined. | The manifest, or the user. |
| `Denied::Unavailable(why)` | Granted, and this host cannot serve it. | Nobody. Say so and carry on. |
| `Denied::Malformed(why)` | The request itself was not well formed — a path that climbs out. | The widget's author. |

A widget that disappears when declined is a worse answer than one that shows
its last value and says it is stale:

```rust
let time = cx.format_time(now).unwrap_or_else(|_| "--:--".to_string());
```

## Rules worth knowing

**`fs:<subpath>`** — inside the widget's own data directory only. A symlink
cannot lead out of it. 4 MB per file, 32 MB per widget.

**`net:<host>`** — HTTPS only, to exactly that host. A redirect to a different
host is refused, because otherwise a grant for one name is a grant for
wherever that name points. `net:*.example.com` covers subdomains of
`example.com`.

**`link:<host>`** — `https` only, to that host, checked at the click rather
than when the view was built. The widget cannot open anything on its own.

**`secrets`** — a keychain namespaced to the widget, with no argument through
which to name another widget's. 64 keys of 64 KiB. `install` says what the
keychain on this machine actually is at the moment you are asked to approve.

**`command:<name>`** — no shell anywhere, so nothing in the arguments can
become a second command; almost no environment; no standard input; bounds on
time and output. But a command can do whatever that command can do:
`command:sh` is a request for arbitrary code execution wearing a small word.
Ask for the smallest command that answers your question.

**`metrics`** — CPU, memory, disk, battery and network, under one grant,
because "this widget reads how the machine is doing" is one sentence. A
machine with no battery answers `Ok(None)` — its normal case, not an error.

## Not capabilities

Some things need no grant because they reach nothing outside the sandbox:

* **The wall clock** — `now_seconds()`. Knowing the time reveals nothing.
* **Settings** — `cx.setting(key)`, the widget's own declared configuration.
* **Images** — `register_image(png)`, bytes the widget already had, bounded to
  1 MiB, 1024 px a side, 16 at once.
* **Logging** — `cx.log(line)`, into the desktop's log.

The full surface, with every limit, is the [API reference](../reference/api.md).
