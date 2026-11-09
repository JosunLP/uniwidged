# Quick start

Five minutes from nothing to your own widget on the desktop. You need:

- a 1.x `uniwidged`, [installed](./installation.md) — `uniwidged --version`
  says which. Hosts before 1.0.0 cannot find a widget's manifest next to a fresh
  build, so the install below fails on them;
- [Rust](https://www.rust-lang.org/tools/install) 1.95 or newer, through
  `rustup` (`rustc --version` says which; `rustup update` gets a newer one);
- `git`, to fetch the reference widgets.

## 1. Install a widget

Reference widgets live in the `widgets/` folder of the repository. Build them
from the release you installed, so the widget speaks the same interface as your
host. With no checkout yet:

```sh
git clone --branch v1.0.0 https://github.com/JosunLP/uniwidged
cd uniwidged
```

If you built `uniwidged` from source, go to the root of that checkout instead.
Then build the clock and install it:

```sh
cd widgets/clock
rustup target add wasm32-wasip2     # for the toolchain this directory uses
cargo build --release --target wasm32-wasip2
uniwidged install target/wasm32-wasip2/release/clock_widget.wasm --allow-unsigned
```

`install` finds the crate's `widget.toml`, prints **what the widget asks for** —
here `timer` and `locale` — and records the grant. To decline a capability,
remove it from `granted` in the configuration file; the widget keeps working
and is told `NotGranted` for that one.

## 2. Start the desktop

```sh
uniwidged run
```

The first line says which shell it got — `Wayland (wlr-layer-shell, bottom
layer)` or `X11` — and the clock appears. `run` stays in the foreground, so
**open a second terminal** for everything below: each command reaches the
desktop that is already up, and you will see every change appear without
restarting it. (With nothing installed, `run` says so and exits — that is why
the widget came first.)

::: tip Running it in the background
`uniwidged autostart enable` starts it with your session from now on.
`uniwidged status` and `uniwidged log -f` tell you what it is doing when it has
no terminal.
:::

::: info Why `--allow-unsigned`?
A widget you did not write must be signed by a key this machine trusts, and the
signature covers the manifest as well as the code. Your own builds are not
signed, so you say so explicitly. See [Finding widgets](./registries.md) for
installing signed widgets from a registry.
:::

## 3. Write your own

```sh
uniwidged new hello && cd hello
cargo test                                     # your logic, natively
cargo build --release --target wasm32-wasip2   # the component
uniwidged install target/wasm32-wasip2/release/hello.wasm --allow-unsigned
```

The scaffold already builds, draws, responds to a click and has its own tests.
Outside a checkout of this repository, the scaffold depends on the SDK by git
at the release tag of your `uniwidged` binary; inside one, by path. Open
`src/lib.rs`:

```rust
impl Widget for Hello {
    fn view(&mut self, _cx: &Cx) -> View {
        panel(NodeId::named("root"))
            .gap(2)
            .child(text(self.greeting.clone()).role(Role::Title))
            .child(text(self.clicks.to_string()).role(Role::Display))
            .child(
                chip("Click me")
                    .accent(Accent::Brand)
                    .on_click(NodeId::named("button"))
                    .label("Count one more"),
            )
            .view()
    }

    fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {
        match event {
            Event::Clicked(id) if *id == NodeId::named("button") => {
                self.clicks += 1;
                true // the view would now differ
            }
            // …
        }
    }
}
```

Edit it, rebuild, and `uniwidged install` again — the running desktop swaps in
the new version.

## 4. Make it yours

```sh
uniwidged configure hello --set greeting="Guten Morgen"
uniwidged move hello --x 64 --y 64
uniwidged lock hello
```

Or drag and resize it with the pointer. Placement is remembered per monitor,
by monitor *name*, so it survives docking and unplugging.

## 5. See what is happening

```sh
uniwidged status   # what is on the screen, and where
uniwidged list     # what is installed, and what it was granted
uniwidged log -f   # what the desktop is saying
```

## Where next

* [Writing a widget](./writing-a-widget.md) — the full guide.
* [Capabilities](./capabilities.md) — asking for the network, files, secrets
  or commands.
* [Running the desktop](./desktop.md) — everything about the host itself.
