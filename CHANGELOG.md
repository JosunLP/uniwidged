# Changelog

What changed, for somebody who upgrades. The commit messages are the long
version — each milestone is one commit with its reasoning and what was found
measuring it — and `ROADMAP.md` is where the design decisions live. This is the
short one.

The widget ABI is versioned separately from the binary: a widget built against
a different ABI is refused at instantiation, in a sentence naming both numbers.
`uniwidged --version` prints both.

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
