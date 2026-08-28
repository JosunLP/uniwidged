# Provenance and licensing

uniwidged is licensed under **MIT OR Apache-2.0**, at your option. See
[`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).

This matters more than usual for a widget framework: the licence a framework
carries is the licence its users inherit. A GPL framework would mean every
widget that links against it is GPL too. uniwidged is permissive so that a
widget author may pick any licence they like, proprietary included.

## Code adopted from Ephemeris

The platform layer of uniwidged (`uniwidged-shell`) derives from
[Ephemeris](https://github.com/JosunLP/Ephemeris), a desktop calendar widget
published under GPL-3.0-or-later. What is adopted is the desktop-layer window
integration: the Win32, AppKit, `wlr-layer-shell` and X11/EWMH code that makes
a window behave like a desktop widget rather than an application window.

That code is used here under a **relicence by its copyright holder**, not under
the GPL. The basis for that:

* The Git history of Ephemeris (42 commits, verified 2026-08-27) records
  exactly one human author, Jonas Pfalzgraf, under two identities —
  `Jonas Pfalzgraf <gh@flausch-code.de>` (29 commits) and
  `JonasPfalzgraf <20913954+JosunLP@users.noreply.github.com>` (9 commits).
* The remaining 4 commits are `dependabot[bot]` dependency bumps, which touch
  `Cargo.lock` and workflow files and contribute no copyrightable expression to
  the source adopted here.
* Ephemeris has no Contributor Licence Agreement and no third-party
  contributions to the adopted files, so the sole copyright holder is free to
  license the same work under different terms.

Ephemeris itself remains GPL-3.0-or-later. Relicensing a copy for use here does
not change the licence of the upstream project, and no GPL obligation attaches
to uniwidged or to widgets built on it.

Where a file is a close adaptation of Ephemeris code, it says so in its module
documentation, naming the upstream file. This is a courtesy to readers rather
than a licence requirement.

## Code adopted from Ferrum UI

The rendering, layout, text and accessibility layers build on
[Ferrum UI](https://github.com/j-pfalzgraf/ferrumui) (MIT OR Apache-2.0), by
the same author as this project.

It was a path dependency on a sibling checkout until 2026-08-28, and is now
**adopted rather than depended on**: uniwidged builds from crates.io alone, so
that cloning this repository and running `cargo test` needs nothing else on the
machine, and so that a widget author is not asked to obtain a second — and
currently private — repository to build the SDK they compile against.

What was actually used was one crate of the nine, `ferrum-layout`: a facade
over [taffy](https://github.com/DioxusLabs/taffy). It now lives at
`crates/uniwidged-compose/src/layout/boxes.rs`, trimmed to the box model a
uniwidged view can describe — grid and scroll containers, margins and the
space-distribution modes `uniwidged-view` has no word for are gone. Both
licences permit this; the same person holds the copyright either way, and the
file names its origin.

`ferrum-core`, `ferrum-render` and `ferrum-text` were declared in the workspace
manifest and never used by any crate in it. They are gone rather than vendored.
The Parley→Vello glyph bridge in `uniwidged-compose::text` follows the
technique in `ferrum-text` and says so where it does, but shares no code with
it: that module exists precisely because the wrapper exposed neither font
weight nor tabular figures.

## The bundled test font

`crates/uniwidged-compose/tests/fonts/DejaVuSans.ttf` is included so that the
golden-image suite renders the same text on every machine. A snapshot taken with
whatever fonts a developer happens to have installed is a snapshot of that
machine rather than of this code, and would differ from CI for reasons neither
could act on.

DejaVu Sans is under the Bitstream Vera licence (permissive; the DejaVu changes
are public domain) — see `DejaVuSans.LICENSE` beside it. It is a **test**
dependency: nothing that ships to a user contains it, and uniwidged uses the
desktop's own UI font at runtime.

## The TLS stack

The `net` capability is served through [ureq](https://github.com/algesten/ureq)
over [rustls](https://github.com/rustls/rustls) — twenty-six crates, all
`Apache-2.0`, `MIT` or `ISC`, which `cargo deny` accepts without an exception.
That was the condition for taking the dependency at all.

rustls rather than the system OpenSSL: this process runs other people's code,
and a memory-safe TLS implementation is worth more here than a smaller binary.

One licence needed adding to `deny.toml` for it: `webpki-roots`, the Mozilla
root certificate bundle, ships under `CDLA-Permissive-2.0`. That is a *data*
licence — the crate is a list of certificates rather than code — and it is
permissive, with no copyleft and no attribution requirement. It is allowed
explicitly rather than by loosening the policy, so the next licence to appear
still has to be read by somebody.

## Third-party dependencies

The Linebender stack (Vello, Parley, kurbo, peniko), taffy, AccessKit, wgpu and
Wasmtime are all Apache-2.0 / MIT / BSD. `cargo deny check` enforces that no
copyleft licence enters the dependency tree — see [`deny.toml`](deny.toml).
