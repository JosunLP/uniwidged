# uniwidged-guest

The SDK for writing a [uniwidged](https://github.com/JosunLP/uniwidged) desktop
widget in Rust. A widget is a WebAssembly component (`wasm32-wasip2`) that
*describes* what it shows; the host lays it out, draws it below every window,
and decides what it may touch.

```toml
[dependencies]
uniwidged-guest = "1.1"
uniwidged-core = "1.1"
uniwidged-view = "1.1"
```

```rust,ignore
use uniwidged_guest::prelude::*;

#[derive(Default)]
struct Clock;

impl Widget for Clock {
    fn view(&mut self, cx: &Cx) -> View {
        panel(NodeId::named("clock"))
            .child(text(cx.format_time(now()).unwrap_or_default()).role(Role::Display))
            .view()
    }

    fn wake(&self) -> Wake {
        Wake::At(next_minute())
    }
}

uniwidged_guest::export!(Clock);
```

`uniwidged new my-widget` writes a project like this that already builds,
draws, responds to a click and has a test. Everything except `Cx` works
natively, so a widget's logic is unit-tested with a plain `cargo test`.

* [The authoring guide](https://josunlp.github.io/uniwidged/guide/)
* [The API reference](https://josunlp.github.io/uniwidged/reference/api)
* [ABI & stability](https://josunlp.github.io/uniwidged/reference/abi) — a
  widget built with any 1.x SDK runs on every later 1.x host.

The interface itself is `wit/uniwidged.wit`, shipped in this crate. It is a
WIT world, so a widget does not have to be Rust: the repository has a C SDK
built on the same file.

Licensed under MIT OR Apache-2.0.
