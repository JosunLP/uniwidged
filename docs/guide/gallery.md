# Reference widgets

Eight widgets live in the repository's
[`widgets/`](https://github.com/JosunLP/uniwidged/tree/main/widgets) folder.
Each is small enough to read in a sitting, declares **only** what it uses — a
test asserts that list — and is built and tested in CI for `wasm32-wasip2` on
every change. Between them they cover `timer`, `locale`, `metrics` (one per
kind of answer it gives), `fs`, `command` and `secrets`. There is no worked
example for `net`, `link` or `measure` yet; [Writing a widget](./writing-a-widget.md)
and the [API reference](../reference/api.md) cover them.

<WidgetGallery />

## Building one

```sh
# no checkout yet: the tag of the release you installed
git clone --branch v1.1.0 https://github.com/JosunLP/uniwidged && cd uniwidged
# from the root of the checkout:
cd widgets/system-load
cargo test                                     # natively — no WASM toolchain needed
cargo build --release --target wasm32-wasip2
uniwidged install target/wasm32-wasip2/release/system_load_widget.wasm --allow-unsigned
```

## What they look like

These are not screenshots of a mock-up: they are the golden images the render
pipeline is tested against, rendered headlessly in CI.

<div class="uw-golden">
  <figure><img src="/images/golden/charts-dark.png" alt="Sparklines for CPU and memory, dark theme" /><figcaption>sparkline · dark</figcaption></figure>
  <figure><img src="/images/golden/charts-light.png" alt="The same sparklines, light theme" /><figcaption>sparkline · light</figcaption></figure>
  <figure><img src="/images/golden/roles-dark.png" alt="Every text role, dark theme" /><figcaption>roles</figcaption></figure>
  <figure><img src="/images/golden/accents-dark.png" alt="Every accent, dark theme" /><figcaption>accents</figcaption></figure>
  <figure><img src="/images/golden/rtl-arabic.png" alt="A right-to-left layout with Arabic text" /><figcaption>right-to-left</figcaption></figure>
</div>

<style>
.uw-golden { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 14px; margin: 20px 0; }
.uw-golden figure { margin: 0; padding: 10px; border-radius: 14px; background: var(--uw-glass-bg); border: 1px solid var(--uw-glass-border); text-align: center; }
.uw-golden img { margin: 0 auto; border-radius: 8px; }
.uw-golden figcaption { margin-top: 6px; font-family: var(--vp-font-family-mono); font-size: 11.5px; color: var(--vp-c-text-3); }
</style>
