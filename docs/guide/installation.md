# Installation

uniwidged runs on **Linux** (Wayland and X11), **Windows 10 and 11**, and
**macOS 11 or later**. On Linux the binary picks the right window system at
start-up and says which it chose; on the other two there is only one.

## Requirements

| | |
|---|---|
| A compositor | **Wayland:** one that implements `wlr-layer-shell` — sway, Hyprland, river, labwc, KDE Plasma, Wayfire and others. **X11:** any window manager with a compositing manager running, for per-pixel alpha. **Windows and macOS:** always there (DWM, the window server). |
| A GPU | **Linux:** anything with a Vulkan, OpenGL ES or GL driver that wgpu supports; a software driver (lavapipe) works too. **Windows:** a DirectX 12 adapter — WARP, the software one every Windows has, counts. **macOS:** Metal, which every supported Mac has. |
| Fonts | Whatever fontconfig finds. The host uses your session's fonts. |

Not sure what your session offers? After installing, run:

```sh
uniwidged doctor
```

It reports the window system, the alpha path, the compositor's capabilities and
what every machine counter answers here — in the same words the start-up log
uses.

## From a release package

Every tagged release on GitHub carries a `.deb`, a Windows installer and a
macOS bundle, each built and inspected in CI, with their SHA-256 checksums in
`SHA256SUMS`.

::: code-group

```sh [Debian / Ubuntu]
base=https://github.com/JosunLP/uniwidged/releases/download/v1.1.0
curl -LO "$base/uniwidged_1.1.0_amd64.deb" -LO "$base/SHA256SUMS"
sha256sum --ignore-missing -c SHA256SUMS && \
  sudo apt install ./uniwidged_1.1.0_amd64.deb
```

```powershell [Windows]
# Per-user: installs under %LOCALAPPDATA%\Programs, adds itself to your PATH,
# and never asks for an administrator.
$base = "https://github.com/JosunLP/uniwidged/releases/download/v1.1.0"
Invoke-WebRequest "$base/uniwidged-1.1.0-x64.msi" -OutFile uniwidged-1.1.0-x64.msi
Invoke-WebRequest "$base/SHA256SUMS" -OutFile SHA256SUMS
$want = (Select-String -Path SHA256SUMS -Pattern 'uniwidged-1.1.0-x64.msi').Line.Split(' ')[0]
$have = (Get-FileHash uniwidged-1.1.0-x64.msi -Algorithm SHA256).Hash.ToLower()
if ($want -and $have -eq $want) {
    msiexec /i uniwidged-1.1.0-x64.msi
} else {
    Write-Error "checksum mismatch or missing: do not install this file"
}
```

```sh [macOS]
base=https://github.com/JosunLP/uniwidged/releases/download/v1.1.0
curl -LO "$base/uniwidged-1.1.0-macos.zip" -LO "$base/SHA256SUMS"
# Exactly this file's line, so a missing entry fails rather than passing on
# whatever else the list holds.
grep ' uniwidged-1.1.0-macos.zip$' SHA256SUMS | shasum -a 256 -c - && \
  mkdir -p ~/Applications && \
  ditto -x -k uniwidged-1.1.0-macos.zip ~/Applications
# The command line, from the bundle:
mkdir -p ~/.local/bin
ln -s ~/Applications/uniwidged.app/Contents/MacOS/uniwidged ~/.local/bin/uniwidged
```

`~/Applications` needs no administrator. To install for every account on the
Mac, extract into `/Applications` instead — which an administrator account can
write to.

```sh [Arch Linux]
# packaging/PKGBUILD builds the tagged release from source
git clone https://github.com/JosunLP/uniwidged
cd uniwidged/packaging
makepkg -si
```

:::

The `.deb` installs the binary, the manual page (`man uniwidged`), a session
desktop entry, and completions for bash, zsh and fish. The Windows installer
installs the binary and a Start-menu entry that starts the desktop. The macOS
bundle is a universal binary for Apple Silicon and Intel, with no Dock tile.

::: warning Signing
A release is signed only where the project's signing keys were available to
the release workflow, and its notes say which. An **unsigned** Windows
installer makes SmartScreen warn before it runs; a macOS bundle that is **not
notarised** is refused by Gatekeeper when it came from a download — remove the
quarantine flag with `xattr -d com.apple.quarantine ~/Applications/uniwidged.app`
if you have checked `SHA256SUMS` and trust it.
:::

## From source

You need a Rust toolchain of **1.95 or newer**, and what Rust itself needs to
link: on **Windows**, the Visual Studio C++ build tools (which `rustup` offers
to install for the MSVC toolchain); on **macOS**, the Xcode Command Line Tools
(`xcode-select --install`). On **Linux** you also need three system
libraries:

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

The SDK is on crates.io as [`uniwidged-guest`](https://crates.io/crates/uniwidged-guest),
so a widget depends on it like on anything else. Widgets can also be written in
C, with wasi-sdk — see [`sdks/c`](https://github.com/JosunLP/uniwidged/tree/main/sdks/c).

## Shell completions

The packages install these for you. From source:

::: code-group

```sh [bash]
mkdir -p ~/.local/share/bash-completion/completions
uniwidged completions bash > ~/.local/share/bash-completion/completions/uniwidged
```

```sh [zsh]
mkdir -p ~/.zfunc
uniwidged completions zsh > ~/.zfunc/_uniwidged
# and in ~/.zshrc, before `compinit`:
#   fpath=(~/.zfunc $fpath)
```

```sh [fish]
mkdir -p ~/.config/fish/completions
uniwidged completions fish > ~/.config/fish/completions/uniwidged.fish
```

:::

Completions include the names of your installed widgets.

## Starting with your session

```sh
uniwidged autostart enable
```

This writes an XDG autostart entry on Linux, a `LaunchAgent` on macOS
(`~/Library/LaunchAgents/io.github.josunlp.uniwidged.plist`), and a value under
`HKCU\…\CurrentVersion\Run` on Windows — the one place each platform's own
*startup apps* settings list. It will not overwrite one you wrote by hand. `uniwidged autostart status` says what is there; `disable` removes ours.

## Checking it worked

```sh
uniwidged --version   # uniwidged 1.1.0 (widget ABI 9)
uniwidged doctor
```

Continue with the [quick start](./quick-start.md).
