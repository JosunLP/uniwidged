# Introduction

**uniwidged** is a desktop widget framework. One host process owns one GPU
device and every widget's surface. Widgets sit **below your windows and above
your wallpaper**, composited per pixel against it — not floating in a grey
rectangle. Each widget is a **WebAssembly component**: sandboxed, installable at
runtime, and written in any language that targets the component model.

<div class="uw-tldr">

| | |
|---|---|
| **For people** | Clocks, system meters, forecasts and dashboards on the desktop that cost no CPU while nothing changes, and that can reach only what you approved. |
| **For widget authors** | A small Rust SDK, a manifest, and a host that does layout, text, theming, accessibility and hit-testing for you. |
| **Platforms** | Linux — Wayland (`wlr-layer-shell`) and X11; Windows (DirectComposition); macOS (AppKit and Metal). Windows and macOS are new in 1.1. |
| **Licence** | MIT OR Apache-2.0. |

</div>

## The problem it solves

A desktop widget is on screen *all day*. That changes which properties matter:

1. **What it costs while nothing is happening.** Most widget engines run a
   render loop or a timer per widget. uniwidged has neither: every widget says
   when it next wants to be called, the host sleeps until the earliest of those,
   and a view that has not changed produces no GPU work at all.
2. **What it can reach while you are not looking.** A widget is somebody else's
   code running in your session. Here it gets no files, no network and no
   commands unless its manifest named them — by folder, host and command — and
   you agreed.
3. **Whether it looks like it belongs.** Widgets describe *what* they show —
   roles like `Display` or `Caption`, accents like `Critical` — and the host
   decides *how*. A screen of widgets by twenty authors looks like one product,
   and every one of them follows your theme, contrast and scaling.

## How it fits together

```text
 ┌────────────── your desktop session ──────────────┐
 │  windows                                          │
 │ ───────────────────────────────────────────────── │
 │  uniwidged host  (one process, one GPU device)    │
 │    ├─ scheduler     sleeps until the next wake    │
 │    ├─ compose       view → layout → text → scene  │
 │    ├─ render        Vello + wgpu, many surfaces   │
 │    ├─ shell         layer-shell / X11 below-state │
 │    └─ runtime       Wasmtime, one sandbox each    │
 │         ├─ clock.wasm        timer, locale        │
 │         ├─ system-load.wasm  timer, metrics       │
 │         └─ weather.wasm      net:api.example.com  │
 │ ───────────────────────────────────────────────── │
 │  wallpaper                                        │
 └───────────────────────────────────────────────────┘
```

A widget never draws. It returns a small **view tree** — panels, rows, text,
meters, sparklines — and the host lays it out with taffy, shapes the text with
Parley, paints it with Vello on wgpu, builds an AccessKit tree from the same
pass, and puts it on a desktop-layer surface. [Core concepts](./concepts.md)
walks through each step.

## Where it comes from

uniwidged is the union of two projects. [**Ephemeris**](https://github.com/JosunLP/Ephemeris)
worked out how a window becomes a *desktop widget* on every platform — that is
`uniwidged-shell`. [**Ferrum UI**](https://github.com/j-pfalzgraf/ferrumui) is a
GPU-rendered UI stack on Vello, wgpu, Parley, taffy and AccessKit — that is
where the rendering techniques come from. Both are adopted rather than
depended on: `git clone && cargo test` works with nothing else on the machine.

What is new is the seam between the two, and the sandbox boundary that lets
somebody else write a widget.

## What it is not

* **Not an application framework.** That is Ferrum UI.
* **Not an HTML/CSS/JS runtime.** Widgets do not bring their own renderer.
* **Not a canvas.** A widget cannot put a pixel where it wants one — and that
  is why they all look right together.
* **Not a panel or status bar.** The shell layer would serve one; it is
  somebody else's project.

## Next steps

* [Install it](./installation.md), then follow the [quick start](./quick-start.md).
* Read [core concepts](./concepts.md) for the ideas behind the API.
* Jump straight to [writing a widget](./writing-a-widget.md).

<style>
.uw-tldr table { margin: 24px 0; }
.uw-tldr td:first-child { white-space: nowrap; font-weight: 600; }
</style>
