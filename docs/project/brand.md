# Brand

<div class="uw-brand-hero">
  <img src="/logo.svg" alt="The uniwidged mark" width="128" height="128" />
  <div>
    <img class="light-only" src="/wordmark-light.svg" alt="uniwidged" width="420" />
    <img class="dark-only" src="/wordmark-dark.svg" alt="uniwidged" width="420" />
  </div>
</div>

## The idea

The mark is a **u** whose right arm is also the stem of an **i** — *ui*, and
*uni*. The dot over it is the one accent a widget gets, and the marker on the
latest point of a sparkline. The gradient behind is the wallpaper; the frosted
square on it is the desktop layer, composited per pixel over what is beneath —
which is the thing this project exists to do well.

## Colours

<div class="uw-swatches">
  <div><i style="background:#7C5CFF"></i><b>Violet</b><code>#7C5CFF</code></div>
  <div><i style="background:#4F7BFF"></i><b>Blue</b><code>#4F7BFF</code></div>
  <div><i style="background:#14C8B0"></i><b>Teal</b><code>#14C8B0</code></div>
  <div><i style="background:#FFB547"></i><b>Amber</b><code>#FFB547</code></div>
  <div><i style="background:#0C0A1D"></i><b>Ink</b><code>#0C0A1D</code></div>
</div>

Amber is the dot: used sparingly, once per composition.

## Type

Headings are set in **Space Grotesk**, text in **Inter**, code in **JetBrains
Mono**.

## Files

| File | Use |
|---|---|
| [`logo.svg`](/logo.svg) | The mark, in colour. Also the favicon. |
| [`logo-mono.svg`](/logo-mono.svg) | The mark in `currentColor`, for one-colour contexts. |
| [`wordmark-light.svg`](/wordmark-light.svg) · [`wordmark-dark.svg`](/wordmark-dark.svg) | Mark and name, for light and dark backgrounds. |
| [`logo-512.png`](/logo-512.png) | Raster mark. |
| [`og-image.png`](/og-image.png) | The 1280×640 social card. |

The sources and the script that renders the rasters are in
[`assets/`](https://github.com/JosunLP/uniwidged/tree/main/assets). The marks
are licensed like the code; please don't use them to suggest endorsement by
the project.

<style>
.uw-brand-hero { display: flex; flex-wrap: wrap; align-items: center; gap: 32px; padding: 32px; margin: 24px 0; border-radius: 20px; background: var(--uw-glass-bg); border: 1px solid var(--uw-glass-border); }
.dark .light-only, html:not(.dark) .dark-only { display: none; }
.uw-swatches { display: grid; grid-template-columns: repeat(auto-fill, minmax(130px, 1fr)); gap: 12px; margin: 20px 0; }
.uw-swatches div { display: flex; flex-direction: column; gap: 4px; padding: 10px; border-radius: 14px; border: 1px solid var(--vp-c-divider); }
.uw-swatches i { display: block; height: 56px; border-radius: 10px; box-shadow: inset 0 0 0 1px rgba(255,255,255,.1); }
.uw-swatches code { font-size: 12px; }
</style>
