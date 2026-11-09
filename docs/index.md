---
layout: home
title: Desktop widgets, done properly

hero:
  name: uniwidged
  text: Desktop widgets that cost nothing when nothing happens.
  tagline: One GPU-rendered host, many sandboxed WASM widgets — composited per pixel against your wallpaper, below your windows, by people who never had to agree on a colour.
  actions:
    - theme: brand
      text: Get started
      link: /guide/quick-start
    - theme: alt
      text: Write a widget
      link: /guide/writing-a-widget
    - theme: alt
      text: GitHub
      link: https://github.com/JosunLP/uniwidged

features:
  - icon: 🪶
    title: Zero-cost idle
    details: No idle loop, no idle frame. The scheduler sleeps until the earliest deadline any widget asked for — eight clocks cost one wake-up, not eight.
    link: /guide/concepts#the-wake-model
    linkText: The wake model
  - icon: 🧩
    title: Sandboxed WASM components
    details: Every widget is a wasm32-wasip2 component with a memory cap, a 16 ms deadline and no ambient authority. It reaches only what its manifest declared and you approved.
    link: /guide/capabilities
    linkText: Capabilities
  - icon: 🎨
    title: You describe, the host draws
    details: Roles and accents instead of fonts and colours. Vello, Parley and taffy do the rest — shaped text, Oklch colour, real shadows — so twenty authors look like one product.
    link: /guide/concepts#you-describe-the-host-decides
    linkText: How views work
  - icon: 🔏
    title: Signed, checked, then trusted
    details: Ed25519 signatures over the component and its manifest. A registry is one signed TOML file on a static host — and downloading is still not trusting.
    link: /guide/registries
    linkText: Registries
  - icon: 🖥️
    title: Wayland and X11
    details: A wlr-layer-shell surface on the bottom layer, or a 32-bit _NET_WM_STATE_BELOW window. Multi-monitor placement that survives docking and unplugging.
    link: /guide/desktop
    linkText: Running the desktop
  - icon: ♿
    title: Accessible by construction
    details: The same pass that lays a view out builds its AccessKit tree. Interaction is reported by node id, never by coordinate, so a screen reader sees what you see.
    link: /reference/api#identity-and-interaction
    linkText: Ids and interaction
---

<div class="uw-section">
<div class="uw-kicker">Why it exists</div>
<h2 class="uw-title">Three promises, each with a number behind it</h2>
<p class="uw-lede">A desktop widget is on screen all day. That changes what matters: what it costs while nothing changes, what it can reach while you are not looking, and whether it still looks right next to everything else.</p>
<Pillars />
</div>

<div class="uw-section">
<div class="uw-kicker">A whole widget</div>
<h2 class="uw-title">Thirty lines of Rust, one manifest</h2>
<p class="uw-lede">No colours, no fonts, no pixels, no coordinates. A widget says <code>Role::Display</code>, and the theme decides what that is — legible against <em>this</em> wallpaper at <em>this</em> contrast setting.</p>

<div class="uw-split">
<div>

```rust
use uniwidged_guest::prelude::*;

#[derive(Default)]
pub struct Clock;

impl Widget for Clock {
    fn view(&mut self, cx: &Cx) -> View {
        let now = now_seconds();
        // A refusal is shown, not hidden.
        let time = cx.format_time(now).unwrap_or_else(|_| "--:--".into());
        let date = cx.format_date(now).unwrap_or_default();

        panel(NodeId::named("clock"))
            .align(Align::Center)
            .child(text(time).role(Role::Display))
            .child(text(date).role(Role::Caption).muted())
            .view()
    }

    // The top of the next minute — not "in 60 seconds" —
    // so it stays right across suspend and NTP steps.
    fn wake(&self) -> Wake {
        Wake::at_next_multiple_of(60, now_seconds())
    }
}

uniwidged_guest::export!(Clock);
```

</div>
<div>

```toml
# widget.toml — everything a user is asked to approve
name = "clock"
version = "1.0.0"
description = "The time and the date, in your locale"
capabilities = ["timer", "locale"]
memory-mb = 8
```

```sh
uniwidged new clock && cd clock
cargo build --release --target wasm32-wasip2
uniwidged install target/wasm32-wasip2/release/clock.wasm \
  --allow-unsigned
uniwidged run
```

</div>
</div>
</div>

<div class="uw-section">
<div class="uw-kicker">Rendered, not mocked</div>
<h2 class="uw-title">Pixels from the test suite</h2>
<p class="uw-lede">These are golden images: rendered headlessly on a software Vulkan device in CI on every change, and compared to within one 8-bit unit. The same widget, three themes — nothing in the widget changed.</p>

<div class="uw-shots">
  <figure><img src="/images/golden/widget-dark.png" alt="A system widget rendered in the dark theme" width="320" height="200" /><figcaption>dark</figcaption></figure>
  <figure><img src="/images/golden/widget-light.png" alt="The same widget in the light theme" width="320" height="200" /><figcaption>light</figcaption></figure>
  <figure><img src="/images/golden/widget-high-contrast.png" alt="The same widget in high contrast" width="320" height="200" /><figcaption>high contrast</figcaption></figure>
</div>
</div>

<div class="uw-section">
<div class="uw-kicker">Capabilities</div>
<h2 class="uw-title">Sentences a person can agree to</h2>
<p class="uw-lede"><code>net:api.open-meteo.com</code>, not <code>network</code>. A capability names the host, the folder or the command, because “this widget talks to api.open-meteo.com” is something you can say yes to — and “uses the network” is not.</p>
<CapabilityGrid />
</div>

<div class="uw-cta">
<h2>Ready when your desktop is.</h2>
<p>Linux, Wayland or X11. Build it from source in one command, or install the <code>.deb</code> from the latest release.</p>
<span class="uw-cmd">cargo install --path crates/uniwidged-cli</span>
</div>

<style>
.uw-shots { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 16px; margin-top: 24px; }
.uw-shots figure { margin: 0; padding: 14px; border-radius: 18px; background: var(--uw-glass-bg); border: 1px solid var(--uw-glass-border); box-shadow: var(--uw-glass-shadow); text-align: center; }
.uw-shots img { width: 100%; height: auto; border-radius: 10px; image-rendering: auto; }
.uw-shots figcaption { margin-top: 8px; font-family: var(--vp-font-family-mono); font-size: 12px; color: var(--vp-c-text-3); }
</style>
