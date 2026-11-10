# The C SDK

A uniwidged widget written in C. Same interface, same sandbox, same host: a
widget built here is a `wasm32-wasip2` component exactly like one built with
the Rust SDK, and `uniwidged install` cannot tell them apart.

It exists because the interface is a WIT file — [`wit/uniwidged.wit`](../../wit/uniwidged.wit)
— and the claim that it is a contract rather than a Rust API with extra steps
is only worth something once a second language has been held to it. CI builds
the example here and runs it in the real runtime on every change
(`crates/uniwidged-runtime/tests/c_sdk.rs`).

## What you need

* **wasi-sdk** 25 or later — its `clang` targets `wasm32-wasip2` and links a
  component directly. Point `WASI_SDK_PATH` at it (default `/opt/wasi-sdk`).
* **wit-bindgen** 0.61 — `cargo install wit-bindgen-cli --version 0.61.1`.
* `make`.

## Building

```sh
make -C sdks/c                              # the example → sdks/c/build/c-counter.wasm
make -C sdks/c WIDGET=$PWD/my-widget        # every .c file in my-widget/, and its widget.toml
uniwidged install sdks/c/build/c-counter.wasm --allow-unsigned
```

The build puts the manifest beside the component as `c-counter.toml`, which is
where `install` looks first — the same layout a registry download has. Sign it
with `uniwidged sign` before giving it to anybody else.

The bindings (`widget.h`, `widget.c`) are generated from the interface on every
build rather than committed: the interface is the contract, and a generated
copy that drifted from it would be a second contract nobody agreed to.

## Writing one

```c
#include <uniwidged.h>

static int count;

void uw_widget_view(uw_tree *t) {
    uw_node root = uw_panel(t, uw_id("counter"));
    uw_textf(t, root, "%d", count);
    uw_on_click(t, uw_chip(t, root, "+1"), uw_id("plus"));
}

bool uw_widget_on_event(const uw_event *e) {
    if (uw_clicked(e, uw_id("plus"))) { count++; return true; }
    return false;
}
```

`uw_widget_view` is the one function you must define. `uw_widget_start`,
`uw_widget_on_event` and `uw_widget_wake` are optional and default to what the
Rust SDK's do: start, ignore events, never wake.

The rules are the Rust SDK's rules, because they are the interface's:

* **You describe; the host draws.** No colour, no font, no pixel — a role, an
  accent and a layout in theme steps.
* **Ids are names.** `uw_id("plus")` is the same 32-bit FNV-1a as
  `NodeId::named("plus")`, so a click comes back as the id you gave the node.
* **Nothing blocks.** `fetch` and `run-command` return a token and the answer
  arrives as an event; ask for the next wake rather than polling.
* **The tree is the host's** once `uw_widget_view` returns. Do not free it and
  do not keep pointers into it.

[`include/uniwidged.h`](include/uniwidged.h) wraps the tree, the events, the
wake and the common host calls. Everything else — `fetch`, `run-command`,
`read-file`, the keychain, images — is in the generated `widget.h`, which the
header includes, under the names `wit-bindgen` gives the interface
(`uniwidged_widget_host_fetch`, …).
