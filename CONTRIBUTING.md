# Contributing to uniwidged

Thank you for wanting to help. uniwidged is a small codebase with strong
opinions, and this page is how to work with them rather than against them.

## Ground rules

* **Measured, not asserted.** A claim about behaviour — "it does not redraw",
  "the widget stays below windows", "a redirect off the host is refused" — has a
  test behind it, run in CI. If a change makes a claim, it brings the test.
* **The documentation is part of the code.** Most of this codebase is prose
  explaining *why*. Keep module docs true when you change what they describe;
  `cargo doc` runs with `-D warnings`, so a broken intra-doc link fails CI.
* **No widget-visible change without an ABI decision.** See
  [ABI & stability](https://josunlp.github.io/uniwidged/reference/abi): within
  1.x the interface only grows.

## Getting set up

```sh
git clone https://github.com/JosunLP/uniwidged && cd uniwidged
sudo apt install pkg-config libfontconfig1-dev libxcb1-dev   # or your distribution's names
rustup target add wasm32-wasip2
cargo test --workspace
```

Every dependency is on crates.io. Some suites need more than a compiler and say
so rather than skipping quietly — the README's *Building* table lists them. To
run what CI runs:

```sh
sudo apt install xvfb sway x11-xserver-utils mesa-vulkan-drivers openssl
UNIWIDGED_REQUIRE_X11=1 UNIWIDGED_REQUIRE_WAYLAND=1 UNIWIDGED_REQUIRE_NET=1 \
  cargo test --workspace
```

## Before you open a pull request

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo test --workspace
```

And, if you touched a widget in `widgets/`, in that widget's directory:

```sh
cargo fmt --check && cargo clippy --all-targets && cargo test
cargo build --release --target wasm32-wasip2
```

### Golden images

A change to rendering will fail `crates/uniwidged-compose/tests/snapshots.rs`
and write a `*.actual.png` beside the reference. Look at both. If the new one
is right, replace the reference and say in the pull request what changed and
why it is better.

### The documentation site

```sh
cd docs
npm install
npm run dev      # http://localhost:5173/uniwidged/
npm run build
```

## Commits and pull requests

* One logical change per commit. The message says what changed and **why**;
  the existing history is the house style.
* User-visible changes get a line in `CHANGELOG.md` under **Unreleased**, in
  the section for the people it affects — users or widget authors.
* Fill in the pull request template; it is short.

## Reporting bugs

Open an issue with `uniwidged --version`, `uniwidged doctor` and the relevant
part of `uniwidged log`. For a security problem, see [`SECURITY.md`](https://github.com/JosunLP/uniwidged/blob/main/SECURITY.md)
instead — please do not file it publicly.

## Licence

By contributing you agree that your contribution is licensed under
MIT OR Apache-2.0, like the rest of the project, without any additional terms.
