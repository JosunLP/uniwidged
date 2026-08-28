# Packaging uniwidged

What is here builds a package a person can install, from a checkout, with no
network beyond `cargo fetch` and no account anywhere.

| | | |
|---|---|---|
| [`deb/build.sh`](deb/build.sh) | a `.deb` for Debian and Ubuntu | **built in CI on every change** |
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

**MSI/MSIX, and a signed and notarised `.app`.** Both need the platform they
package for, and uniwidged has no Windows shell (M5) and no macOS shell (M6)
yet — so packaging one would be packaging a binary that prints "this build has
no shell for the current platform". The scripts will come with the platforms,
in the same commit as something to run.

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
