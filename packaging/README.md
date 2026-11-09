# Packaging uniwidged

What is here builds a package a person can install, from a checkout, with no
network beyond `cargo fetch` and no account anywhere.

| | | |
|---|---|---|
| [`deb/build.sh`](deb/build.sh) | a `.deb` for Debian and Ubuntu | **built in CI on every change** |
| [`windows/build.ps1`](windows/build.ps1) | a per-user MSI for Windows | **built, installed and run in CI** |
| [`macos/build.sh`](macos/build.sh) | `uniwidged.app`, signed and notarised when it can be | **built and run in CI** |
| [`PKGBUILD`](PKGBUILD) | an Arch package, for the AUR | written, not built here |
| [`uniwidged.1`](uniwidged.1) | the manual page | installed by both |
| [`uniwidged.desktop`](uniwidged.desktop) | the session entry | installed by both |

## What "built in CI" means, and what it does not

The `.deb` job builds the package and then **looks inside it**: the binary is
there, the manual page is there, the desktop entry is there, `dpkg-deb` accepts
the control file, and the version in the package is the version in
`Cargo.toml`. That is a real check, and it is the reason the script is here
rather than in somebody's shell history.

It is not an installation test. Installing a package as root and starting a
desktop session under it is a virtual machine, and a job that pretends
otherwise would be a green tick meaning nothing.

## What is deliberately missing

**MSIX.** It cannot be installed at all without a signing certificate the
machine already trusts, so an MSIX built by a project without one is a package
nobody can open. The MSI covers the same ground and installs without one.

**Flatpak.** It would work, and it is a poor fit for what this program is: a
widget host draws below every window on the desktop, reads the session's fonts,
watches RandR for monitors coming and going, and runs a signature check before
it runs anybody's code. A sandbox around it that has to be opened up for each
of those is a sandbox that ends up holding nothing — and the interesting
sandbox in this project is the one *inside* the process, around each widget.

## Building a `.deb`

```sh
packaging/deb/build.sh                  # writes target/debian/uniwidged_<version>_<arch>.deb
packaging/deb/build.sh --output /tmp    # somewhere else
```

It builds `--release`, so the first run takes a while. It needs `cargo` and
`dpkg-deb`, and says which is missing rather than failing halfway.

## The Arch package

`PKGBUILD` builds from the repository at a tag. Bump `pkgver`, run
`updpkgsums`, and `makepkg -si` locally before pushing anything to the AUR —
the file is written against the documented behaviour of `makepkg` and is not
built by this project's CI, which is exactly why it should not be published
untested.

## The Windows installer

```powershell
dotnet tool install --global wix --version 5.0.2
packaging\windows\build.ps1                 # writes target\windows\uniwidged-<version>-x64.msi
```

Per-user, on purpose: a widget host runs as you and starts with your session,
and installing it for every account would need an administrator for something
that has no reason to ask for one. With `UNIWIDGED_SIGN_CERT` (a `.pfx`) and
`UNIWIDGED_SIGN_PASSWORD` set, the binary and the installer are signed with
`signtool`; without them the installer is built unsigned and says so.

## The macOS bundle

```sh
packaging/macos/build.sh                # this machine's architecture
packaging/macos/build.sh --universal    # Apple Silicon and Intel, as releases are
```

With `MACOS_SIGNING_IDENTITY` set it is signed with the hardened runtime and
[`macos/uniwidged.entitlements`](macos/uniwidged.entitlements) — the two
Wasmtime's compiled code needs, and nothing else; with `MACOS_NOTARY_PROFILE`
too, it is notarised and stapled. Without an identity it is signed ad hoc,
which is enough to run on the machine that built it and not enough for
Gatekeeper to open it from a download.

