# ABI & stability

uniwidged 1.0 makes one promise above all others:

::: tip The 1.x promise
**A widget built against uniwidged 1.0 keeps running on every 1.x release.**
The host/guest interface — `wit/uniwidged.wit`, ABI **9** — is frozen. It may
grow, and it will never shrink or change, before 2.0.
:::

## Three version numbers

| Number | What it versions | Where you see it |
|---|---|---|
| **uniwidged** `1.1.0` | The binary, the host crates and the SDK, together. Semantic versioning. | `uniwidged --version`, `Cargo.toml`, the release tag |
| **Widget ABI** `9` | The interface between host and widget. One integer, raised with every addition. | `uniwidged --version`, `uniwidged_guest::ABI_VERSION` |
| **Widget version** | Your widget, in its manifest. Semantic versioning; orders registry updates. | `widget.toml` |

```sh
$ uniwidged --version
uniwidged 1.1.0 (widget ABI 9)
```

## How the check works

Every widget exports `abi-version`, and the host calls it as soon as the
component is instantiated — before `start` and before the first view. A host
runs every ABI from **9**, the interface 1.0 shipped with, up to its own:

::: info What runs before the check
Instantiating a component can run code of its own — a static constructor, in
languages that have them — before `abi-version` is asked. That code runs under
the same grants and limits as everything else the widget does, and under a deadline, so
an incompatible widget can act within what it was granted before it is
refused — call the host functions its policy permits and, with `fs` or
`secrets`, write data that stays after the refusal. The check protects the host from misreading a widget, not the
widget's grants from the widget.
:::

```text
 widget ABI      host 1.0 (runs 9…9)     host 1.x (runs 9…N)
 ──────────      ───────────────────     ───────────────────
 ≤ 8             refused: rebuild        refused: rebuild
 9               runs                    runs
 10 … N          refused: upgrade host   runs
```

A refusal is a sentence naming both numbers and saying whose move it is:

```text
this widget was built for ABI 8; this host runs ABI 9 to 9, so it needs
rebuilding against a current SDK

this widget was built for ABI 10, which is newer than this host's 9;
upgrading uniwidged will run it
```

A widget built for a newer 1.x is refused in one of two ways. If it **uses** a
host function this host does not have, its component imports that function and
**fails to link** — refused with a component error naming the missing import,
before `abi-version` can be asked. If it uses none of the new functions, it
links and is refused **at the ABI check**, with the sentence above. Either way
the remedy is the same: upgrade uniwidged.

## What may change in 1.x

**Additions only.** Specifically:

* **New host functions** — a new capability, a new counter under `metrics`.
  A widget built against 1.0 does not import them, so it is untouched. A
  widget built against the newer SDK may import them — a component imports
  only what it calls — and if it does, a host too old to provide them refuses
  it at linking; if it does not, the ABI check refuses it instead.
* **New capabilities** in the manifest grammar.
* **New settings types, new limits raised** (never lowered).
* **Host behaviour** — rendering quality, themes, scheduling — that a widget
  cannot observe through the interface.

## What waits for 2.0

* Removing or renaming anything in `wit/uniwidged.wit`.
* Changing a function's parameters or results.
* Changing the **view vocabulary** that crosses the boundary — a new node type
  is a variant a 1.0 host could not decode, so it is a new major.
* Lowering a limit, or tightening a manifest rule that a published widget
  could already be relying on.

## The SDK

`uniwidged-guest` follows the binary's version. Within 1.x it is
**source-compatible**: a widget that compiled against 1.0 compiles against
every 1.x SDK. Additions (like `now_seconds()` in 1.0) are new items; nothing
is removed or changes signature.

## History

The ABI moved eight times before 1.0, each time with a reason. The full table
is at the bottom of the [API reference](./api.md#abi-history). 9 is where it
stopped.
