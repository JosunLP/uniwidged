# Installation

uniwidged 1.0 runs on **Linux**, on both Wayland and X11. The binary picks the
right window system at start-up and says which it chose.

## Requirements

| | |
|---|---|
| A compositor | **Wayland:** one that implements `wlr-layer-shell` — sway, Hyprland, river, labwc, KDE Plasma, Wayfire and others. **X11:** any window manager with a compositing manager running, for per-pixel alpha. |
| A GPU | Anything with a Vulkan, OpenGL ES or GL driver that wgpu supports. A software driver (lavapipe) works too. |
| Fonts | Whatever fontconfig finds. The host uses your session's fonts. |

Not sure what your session offers? After installing, run:

```sh
uniwidged doctor
```

It reports the window system, the alpha path, the compositor's capabilities and
what every machine counter answers here — in the same words the start-up log
uses.

## From a release package

Every tagged release on GitHub carries a `.deb` built and inspected in CI, with
its SHA-256 checksum in `SHA256SUMS`.

::: code-group

```sh [Debian / Ubuntu]
base=https://github.com/JosunLP/uniwidged/releases/download/v1.0.0
curl -LO "$base/uniwidged_1.0.0_amd64.deb" -LO "$base/SHA256SUMS"
sha256sum --ignore-missing -c SHA256SUMS
sudo apt install ./uniwidged_1.0.0_amd64.deb
```

```sh [Arch Linux]
# packaging/PKGBUILD builds the tagged release from source
git clone https://github.com/JosunLP/uniwidged
cd uniwidged/packaging
makepkg -si
```

:::

The package installs the binary, the manual page (`man uniwidged`), a session
desktop entry, and completions for bash, zsh and fish.

## From source

You need a Rust toolchain of **1.95 or newer** and three system libraries.

::: code-group

```sh [Debian / Ubuntu]
sudo apt install pkg-config libfontconfig1-dev libxcb1-dev
```

```sh [Fedora]
sudo dnf install pkgconf-pkg-config fontconfig-devel libxcb-devel
```

```sh [Arch Linux]
sudo pacman -S pkgconf fontconfig libxcb
```

:::

Then:

```sh
git clone https://github.com/JosunLP/uniwidged
cd uniwidged
cargo install --locked --path crates/uniwidged-cli
```

That puts `uniwidged` in `~/.cargo/bin`. Every dependency comes from crates.io;
there is no sibling checkout to arrange.

## For writing widgets

Widgets are built for the `wasm32-wasip2` target. That is the only addition —
no C toolchain, no system libraries and no GPU on the build machine:

```sh
rustup target add wasm32-wasip2
```

## Shell completions

The packages install these for you. From source:

::: code-group

```sh [bash]
uniwidged completions bash > ~/.local/share/bash-completion/completions/uniwidged
```

```sh [zsh]
uniwidged completions zsh > "${fpath[1]}/_uniwidged"
```

```sh [fish]
uniwidged completions fish > ~/.config/fish/completions/uniwidged.fish
```

:::

Completions include the names of your installed widgets.

## Starting with your session

```sh
uniwidged autostart enable
```

This writes an XDG autostart entry. It will not overwrite one you wrote by
hand. `uniwidged autostart status` says what is there; `disable` removes ours.

## Checking it worked

```sh
uniwidged --version   # uniwidged 1.0.0 (widget ABI 9)
uniwidged doctor
```

Continue with the [quick start](./quick-start.md).
