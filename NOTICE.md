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
[Ferrum UI](https://github.com/j-pfalzgraf/ferrumui) (MIT OR Apache-2.0), which
is used as a dependency rather than copied.

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

## Third-party dependencies

The Linebender stack (Vello, Parley, kurbo, peniko), taffy, AccessKit, wgpu and
Wasmtime are all Apache-2.0 / MIT / BSD. `cargo deny check` enforces that no
copyleft licence enters the dependency tree — see [`deny.toml`](deny.toml).
