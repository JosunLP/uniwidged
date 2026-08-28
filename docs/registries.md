# Registries — finding a widget, and publishing one

A registry is **one signed TOML file on a static host**. There is no server to
run, no account to make and no API to call: if you can put two files where
somebody can fetch them over HTTPS, you can publish widgets.

This page is both halves — reading a registry, and being one.

---

## Reading one

```sh
uniwidged registry add https://widgets.example/index.toml --key 3d40…
uniwidged search clock
uniwidged add clock
uniwidged update
```

### `--key` is not optional, and this is why

The key you pass to `registry add` is the key the **index** is signed with. It
says *who publishes this list*. Every later fetch is checked against it, and an
index that does not verify is not read at all — not "read with a warning".

That key is deliberately **not** in your `trusted-keys` file, and a registry
cannot add anything to it. Two different questions are being asked:

| Question | Answered by | Kept in |
|---|---|---|
| Who publishes this list of widgets? | the index signature | `registries.toml`, pinned per registry |
| Who wrote this widget? | the widget's own `.sig` | `trusted-keys`, added with `uniwidged trust` |

So `uniwidged add clock` fetches a widget the registry offered — and then
refuses to install it unless its author is somebody **you** decided to trust.
A registry cannot vouch a widget past that line. Downloading is not trusting.

### What `search` shows you

The name, the version, the description, and the permissions the widget will ask
for, in the same words the consent screen uses:

```
  example (https://widgets.example/index.toml)
    clock 1.0.0
      The time and the date, in your locale
      · update itself on a schedule
      · format dates and times for your locale
```

Those permissions come out of the signed index, and are checked against the
widget's own manifest after the download. If the two disagree — the listing
says `timer` and the manifest asks for your keychain — the install is refused,
even when every signature is valid and every hash matches. What a listing shows
has to be what the runtime enforces, or the listing is a way to lie.

### Updating

```sh
uniwidged update --check     # what is newer
uniwidged update             # take it
uniwidged update clock       # just this one
```

An update keeps **where the widget sat, what it was configured with, and
whether it was switched off**. A new version of a widget is not a new widget.

An update that asks for **more** than the installed version was granted is
reported and not applied:

```
  clock 1.0.0 → 2.0.0 (example)
  not applied: this version of `clock` asks for something the installed one
  did not: secrets.
  That is a decision rather than an update, so `uniwidged add clock` will show
  you the whole list and ask.
```

That is the one thing an unattended command must never do quietly.

### The cache, and being offline

Each index is cached under `~/.config/uniwidged/registry-cache/` with its
signature, exactly as it was served, and **verified again every time it is
read** — a file being local is not a reason to believe it. Commands refresh a
cache older than six hours; `--refresh` forces it and `--offline` forbids it.

A registry that is *unreachable* falls back to the cache with a line saying so.
A registry that answers with a list **nobody signed** does not: that stops the
command. The two are different facts and are treated differently on purpose.

### A registry inside an organisation

If your registry is served under a certificate authority no public trust store
knows, point uniwidged at it:

```sh
export UNIWIDGED_REGISTRY_CA=/etc/ssl/our-ca.pem
```

It replaces the default roots for registry fetches rather than adding to them —
see `Fetcher::trusting`, where that choice is argued.

---

## Publishing one

You need: the widgets, a key to sign the **index** with, and somewhere to serve
files from.

```sh
# One key for the list you publish. Not the key you sign widgets with.
uniwidged keygen --out registry-key

# Everything the index will point at, in one directory.
uniwidged sign clock.wasm --key author-key      # each widget, by its author
uniwidged registry publish \
    --key registry-key \
    --base-url https://widgets.example \
    --out index.toml \
    clock.wasm system-load.wasm

# Upload index.toml, index.toml.sig, and every .wasm/.toml/.sig beside them.
```

`publish` reads each widget the way `install` will: the manifest is parsed, the
digest is the one a signature covers, and the entry it writes carries the
manifest's own capability list. A widget whose manifest does not parse stops
the whole index rather than being listed — a registry that publishes a broken
widget has published a download every client will reject.

Then tell people the one line they need:

```
uniwidged registry add https://widgets.example/index.toml --key <your index key>
```

### The format

```toml
index = "uniwidged-registry-v1"

[[widget]]
name = "clock"
version = "1.0.0"
description = "The time and the date, in your locale"
capabilities = ["timer", "locale"]
component = "https://widgets.example/clock-1.0.0.wasm"
manifest = "https://widgets.example/clock-1.0.0.toml"
signature = "https://widgets.example/clock-1.0.0.wasm.sig"
digest = "a2f1…"
```

| Field | |
|---|---|
| `name` | the widget's own name, held to the same rule the manifest is: it becomes a directory |
| `version` | `major.minor.patch`, with an optional `-rc.1` tail. Build metadata (`+sha`) is refused: it takes no part in ordering, so two entries differing only there could not be told apart |
| `capabilities` | what the manifest declares. Refused if this host cannot parse one — a listing that can print arbitrary text is a listing that can lie |
| `component`, `manifest`, `signature` | `https://` only, checked when the index is read rather than when the download starts |
| `digest` | `sha256` over the component **and** the manifest, domain-separated: the same digest the widget's own signature covers |

`index.toml.sig` sits beside it and is an ordinary uniwidged signature file over
the index text, byte for byte as served.

Every widget may be listed at several versions. `uniwidged add clock` takes the
newest **release**; a pre-release has to be asked for by name
(`uniwidged add clock@2.0.0-rc.1`).

### Serving it

Any static host. The only requirements are HTTPS and that a redirect does not
walk off the host the index is on — a registry is fetched under a grant for
exactly its own hostname, by the same code that enforces that for a widget's
own requests.

### Rotating the index key

There is no revocation protocol, deliberately. Publish the new key, ask people
to run `registry add` again with it, and keep signing with the old one until
they have. A machine that has not heard will refuse the new index loudly rather
than accept it quietly, which is the failure everybody wants of the two.

---

## What a registry deliberately is not

Not a package manager. No dependency resolution, because widgets have no
dependencies. No version ranges, because an entry is an exact version and
nothing asks for a range. No lock file, because the index *is* the lock: a name
and a version resolve to a digest, and the digest is what is checked after the
bytes arrive.

And nothing here runs on its own. Every fetch is one somebody typed a command
to start — a desktop that quietly polls a registry is a desktop that tells
somebody else when it is switched on.
