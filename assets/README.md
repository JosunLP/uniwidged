# Brand assets

| File | Use |
|---|---|
| `logo.svg` | The mark, in colour. Also the favicon. |
| `logo-mono.svg` | The mark in `currentColor`, for one-colour contexts. |
| `wordmark-light.svg` / `wordmark-dark.svg` | Mark and name, for light and dark backgrounds. |
| `logo-512.png`, `logo-180.png` | Raster marks (app icon, `apple-touch-icon`). |
| `social-preview.png` | The 1280×640 card for GitHub's social preview and the docs' `og:image`. |

## The idea

The mark is a **u** whose right arm is also the stem of an **i** — *ui*, and
*uni*. The dot over it is the one accent a widget gets, and the marker on the
latest point of a sparkline. The gradient behind is the wallpaper; the frosted
square on it is the desktop layer, composited per pixel over what is beneath —
which is the thing this project exists to do well.

## Colours

| Token | Hex | |
|---|---|---|
| Violet | `#7C5CFF` | gradient start |
| Blue | `#4F7BFF` | gradient middle, primary brand colour |
| Teal | `#14C8B0` | gradient end |
| Amber | `#FFB547` | the dot — used sparingly, once per composition |
| Ink | `#0C0A1D` | dark backgrounds |

## Regenerating the rasters

```sh
node assets/render.js assets   # needs Playwright with Chromium
cp assets/*.svg docs/public/
cp assets/logo-512.png docs/public/logo-512.png
cp assets/logo-180.png docs/public/apple-touch-icon.png
cp assets/social-preview.png docs/public/og-image.png
```

The marks are licensed like the code (MIT OR Apache-2.0). Please don't use them
to suggest endorsement by the project.
