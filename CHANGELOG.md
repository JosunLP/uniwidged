# Changelog

All notable changes to uniwidged, for somebody who upgrades. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[semantic versioning](https://semver.org/) — since 1.0, with a
[stability promise](https://josunlp.github.io/uniwidged/reference/abi) behind it.

The commit messages are the long version — each milestone is one commit with
its reasoning and what was found measuring it — and `ROADMAP.md` is where the
design decisions live. This is the short one.

The widget ABI is versioned separately from the binary. `uniwidged --version`
prints both, and a widget built against an ABI this host does not run is
refused at instantiation, in a sentence naming both numbers.

## Unreleased

Nothing yet.

## 1.1.0 — 2026-11-09

**Windows and macOS, the SDK on crates.io, and a second SDK.** Every item the
roadmap listed after 1.0 is in this release. Nothing about the widget
interface changed: ABI 9 still, and every 1.0 widget runs unchanged on both new
platforms.

### New platforms

* **Windows 10 and 11.** A widget is a `WS_EX_NOREDIRECTIONBITMAP |
  WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE` popup, kept at `HWND_BOTTOM` on every
  position change, drawn through a DirectComposition swapchain so the
  wallpaper shows through. Per-monitor DPI v2. Its exit criterion — below a
  maximised window, no focus taken by a click, no Alt-Tab entry, a
  premultiplied swapchain — is measured against the real window manager in CI.
* **macOS 11 and later.** A non-activating `NSPanel` at desktop-icon level,
  on every Space and still there under Mission Control, presenting through a
  `CAMetalLayer`; an accessory application with no Dock tile or menu bar.
  Measured the same way, in CI.
* `UNIWIDGED_BACKDROP=system` asks either platform for its own blur behind the
  widget (acrylic, vibrancy). Off by default: it blurs the window's rectangle,
  whose corners the theme did not choose.
* `run`, `reload`, `status`, `list`, `doctor`, `autostart` and links all work
  on both. `reload` is a named event on Windows rather than `SIGHUP`;
  `autostart` writes a `HKCU\…\Run` value or a `LaunchAgent`; links open
  through `ShellExecuteW` and `open`. `run --background` lets go of the
  console on Windows, and is what the autostart entry and the Start-menu
  shortcut use.

### The keychain

* **The key behind `secrets` is held by the OS** on Windows (sealed with DPAPI
  to this account on this machine) and on macOS (an item in the login
  keychain). The store itself is the same encrypted file on all three
  platforms; `uniwidged doctor` says which custody this machine has. Linux is
  unchanged.

### Packaging

* **An MSI**, per-user — no administrator, `%LOCALAPPDATA%\Programs`, the
  binary on your `PATH`, a Start-menu entry — built by `packaging/windows/`.
* **A macOS bundle**, universal, `LSUIElement`, signed with the hardened
  runtime and notarised, built by `packaging/macos/build.sh`.
* Releases attach both beside the `.deb`, with all three in `SHA256SUMS`.
  Signing happens where the release workflow has the keys, and the release
  notes say when it did not.

### For widget authors

* **The SDK is on crates.io**: `uniwidged-guest = "1.1"`, with `uniwidged-core`
  and `uniwidged-view`. `uniwidged new` from a released binary depends on it by
  version instead of by git tag.
* **A C SDK**, in `sdks/c`: a header and one file over the bindings
  `wit-bindgen c` generates, built with wasi-sdk, with a worked example that CI
  builds and runs in the real runtime. Node ids hash the same as the Rust
  SDK's, so `uw_id("plus")` and `NodeId::named("plus")` are the same node.

### Fixed

* On macOS, a swapchain that wgpu calls `PostMultiplied` is composited by Core
  Animation as premultiplied; the blit now premultiplies there, where it would
  otherwise have drawn every translucent pixel too bright.

## 1.0.0 — 2026-11-09

**The first stable release.** The widget interface is frozen, the
documentation has a home, and the first commands a new author runs work as
written. Linux — Wayland and X11 — is the supported platform; Windows and macOS
follow as 1.x minor releases.

### The stability promise

* **ABI 9 is the 1.x interface.** A widget built against uniwidged 1.0 runs on
  every 1.x host. `wit/uniwidged.wit` may grow — new host functions, which an
  older widget simply does not import — and never shrinks or changes before
  2.0. See [ABI & stability](https://josunlp.github.io/uniwidged/reference/abi).
* The host now runs a **range** of ABIs (`ABI_OLDEST..=ABI_VERSION`) rather
  than exactly one, so 1.x releases that add to the interface keep running
  1.0 widgets.
* A refusal says **whose move it is**: an older widget "needs rebuilding
  against a current SDK"; a newer one is "newer than this host's; upgrading
  uniwidged will run it".

### For users

* **Documentation site** at <https://josunlp.github.io/uniwidged/>, built with
  VitePress: introduction, installation, quick start, core concepts, running
  the desktop, registries, troubleshooting, capabilities, a widget gallery, the
  full API and CLI references, configuration files, the security model, the
  architecture, the roadmap and a FAQ. Built on every pull request, deployed
  from `main`.
* **A logo** and a brand: the mark, wordmarks, favicon, social preview card,
  and the script that renders them, in `assets/`.
* `uniwidged help <command>` and `uniwidged <command> --help` show one
  command's usage and flags. Before, `install --help` went looking for a file
  called `--help`.
* An option a command does not know is refused before the command does
  anything: `remove clock --purgee` used to remove the clock without purging
  it.
* The consent screen for `metrics` now names everything the grant covers —
  CPU, memory, disk, battery **and network**. It had not been updated when ABI
  9 added the last two.
* `uniwidged doctor` lists what every machine counter answers here — CPU,
  memory, disk, battery and network — so a meter showing a dash can be traced
  to the machine rather than to the widget.
* Two new reference widgets: `battery` (charge, time left, an hour of history,
  and "on mains power" on a desktop) and `network` (down and up, a minute of
  each on one shared scale). `system-load` gains a disk meter.
* The manual page, the packages and the crate metadata point at the
  documentation and at the repository's real home.

### For widget authors

* **`uniwidged install` finds the manifest of a fresh build.** `cargo build`
  writes `target/wasm32-wasip2/release/<name>.wasm` with no manifest beside it,
  and the guide's next command — `uniwidged install` on that file — was
  refused. `install`, `check` and `sign` now look beside the component, then
  for the `widget.toml` of the crate it was built in (never above the crate
  root), and say which they used. `--manifest <file>` names one explicitly.
* **`uniwidged new` outside a checkout** depends on the SDK by git, at the tag
  of the binary that wrote it, so the scaffold builds on any machine. It used
  to write a crates.io requirement for a crate that is not published there.
* **`now_seconds()`** is in the prelude: the wall clock, which every widget
  that wakes needs and each was writing by hand around `SystemTime`. The
  README's example had been calling it all along.
* **ABI 9.** `cx.battery()`, `cx.disk_load()` and `cx.network_throughput()`
  join `cpu_load` and `memory_load` under the one `metrics` grant.
  `battery` answers `Ok(None)` for a machine without one; `minutes_left` is
  `None` wherever the kernel gives no rate, rather than a guess. Widgets built
  against ABI 8 are refused at instantiation — rebuild against this SDK.

### For contributors

* `CONTRIBUTING.md`, `SECURITY.md` (private vulnerability reporting), issue
  forms and a pull request template.
* A `docs` workflow that builds the site on every change to it and deploys it
  to GitHub Pages from `main`.
* The release workflow publishes this file's section for the tag as the
  release notes.

### Not in this release

* **Windows and macOS** — milestones M5 and M6, each a 1.x minor release.
* **`uniwidged-guest` on crates.io.** The SDK is reached by git at a release
  tag, or by path from a checkout.

## 0.1.0 — 2026-08-31

The first version with a number that means something. A working desktop on
both Linux window systems, every capability served, and all of it measured.

### For users

* `uniwidged run` on Wayland (`wlr-layer-shell`) and X11 (`_NET_WM_STATE_BELOW`
  with a 32-bit visual); the binary picks at start-up and says which.
* Widgets are WASM components, sandboxed, and can do nothing their manifest did
  not declare and the user did not approve: `timer`, `locale`, `measure`,
  `metrics`, `fs:<subpath>`, `net:<host>`, `link:<host>`, `secrets`,
  `command:<name>`.
* `install`, `remove`, `enable`, `disable`, `update`, `configure`, `move`,
  `lock` and `unlock` all reach a desktop that is already running.
* Widgets are dragged, resized and pinned with the pointer, remember which
  monitor they were on, and follow a monitor that is unplugged and returns.
* A registry is one signed TOML file on a static host: `registry add`,
  `search`, `add`, `update`. Downloading is not trusting — the machine's own
  trusted keys still decide.
* `uniwidged status` says what is on the screen; `uniwidged log` says what the
  desktop has been saying, terminal or not; `uniwidged doctor` says what this
  session can and cannot do.
* Shell completions for bash, zsh and fish, written by the binary.
* A `.deb`, built and inspected in CI, and a PKGBUILD.

### For widget authors

* `uniwidged new` scaffolds a widget that builds, draws and responds to a
  click; `docs/writing-a-widget.md` and `docs/api-reference.md` are the guide
  and the reference; five reference widgets, one per capability.
* The view vocabulary: `panel`, `card`, `row`, `column`, `text`, `chip`,
  `icon`, `divider`, `spacer`, `progress`, `sparkline`, `image`. Roles and
  accents rather than fonts and colours.
* Settings declared in the manifest, checked by the host, read with
  `cx.setting`.
* Asynchronous `fetch` and `run_command`, collected by token.
* **ABI 8.** The history is at the bottom of `docs/api-reference.md`.

### Not in this release

* Windows and macOS — milestones M5 and M6 in `ROADMAP.md`.
* A stable ABI. It moves with the interface until 1.0.
