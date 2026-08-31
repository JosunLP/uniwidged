#!/bin/sh
# Builds a .deb of uniwidged from this checkout.
#
# `dpkg-deb` and nothing else: no `cargo-deb`, no `dh`, no build container. The
# package is a binary, a manual page and a desktop entry, and a control file
# that says which shared libraries the binary needs — all of which can be
# written out and checked by reading, which matters more here than the
# convenience of a tool that does it in one line.
#
#   packaging/deb/build.sh [--output DIR] [--target-dir DIR]
#
# Refuses rather than guesses: a missing tool, a version it cannot read, or an
# architecture it has no Debian name for all stop with a sentence.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
output="$root/target/debian"
target_dir="$root/target"

while [ $# -gt 0 ]; do
    case "$1" in
        --output) output=$2; shift 2 ;;
        --target-dir) target_dir=$2; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

need() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "$1 is needed to build a .deb and is not on this machine" >&2
        exit 1
    }
}
need cargo
need dpkg-deb
# The dependencies are asked of the binary rather than written by hand, and
# this is the tool that asks. Without it the control file would have to be
# guessed at, and a guessed `Depends` is a package that installs and then
# cannot start.
need dpkg-shlibdeps

# The version comes from the workspace manifest rather than from a variable
# here: two places to write a version is one place to forget one.
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }

# Debian's name for this machine, which is not always the kernel's.
arch=$(dpkg --print-architecture 2>/dev/null || echo "")
[ -n "$arch" ] || { echo "dpkg could not name this architecture" >&2; exit 1; }

echo "building uniwidged $version for $arch"
cargo build --release --locked --package uniwidged-cli --target-dir "$target_dir"
binary="$target_dir/release/uniwidged"
[ -x "$binary" ] || { echo "$binary was not built" >&2; exit 1; }

staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
# `mktemp` makes a directory only its owner may enter, which is right for a
# temporary directory and wrong for the root of a package: it would install a
# `/` nobody but root could traverse.
chmod 755 "$staging"
install -Dm755 "$binary" "$staging/usr/bin/uniwidged"
install -Dm644 "$root/packaging/uniwidged.1" "$staging/usr/share/man/man1/uniwidged.1"
install -Dm644 "$root/packaging/uniwidged.desktop" \
    "$staging/usr/share/applications/uniwidged.desktop"
install -Dm644 "$root/LICENSE-MIT" "$staging/usr/share/doc/uniwidged/LICENSE-MIT"
install -Dm644 "$root/LICENSE-APACHE" "$staging/usr/share/doc/uniwidged/LICENSE-APACHE"
gzip -9n "$staging/usr/share/man/man1/uniwidged.1"

# The completion scripts are asked of the binary rather than kept as files:
# three shells is three copies of the command list, and the binary is the one
# copy that cannot be out of date. The paths are where each shell's packaging
# on Debian looks — `vendor-completions` for zsh, because `site-functions` is
# the administrator's directory and a package that writes there is rude.
for shell in bash zsh fish; do
    case "$shell" in
        bash) into="$staging/usr/share/bash-completion/completions/uniwidged" ;;
        zsh)  into="$staging/usr/share/zsh/vendor-completions/_uniwidged" ;;
        fish) into="$staging/usr/share/fish/vendor_completions.d/uniwidged.fish" ;;
    esac
    mkdir -p "$(dirname "$into")"
    "$binary" completions "$shell" > "$into"
    chmod 644 "$into"
done

# The shared libraries the binary actually needs, asked of the binary rather
# than listed by hand — and with the *versions* they need, which is the half a
# hand-written list always gets wrong. `libxcb1 (>= 1.12)` rather than
# `libxcb1`: the second installs on a distribution too old to run this and
# fails at the first window.
#
# `dpkg-shlibdeps` insists on a `debian/control` and on being run beside it, so
# it gets a throwaway one. It is the tool every Debian package uses for this
# question and it is worth the two lines of ceremony.
depends="libc6"
shlibs=$(mktemp -d)
mkdir -p "$shlibs/debian"
printf 'Source: uniwidged\n\nPackage: uniwidged\nArchitecture: any\n' > "$shlibs/debian/control"
found=$(cd "$shlibs" && dpkg-shlibdeps -O "$binary" 2>/dev/null | sed -n 's/^shlibs:Depends=//p')
rm -rf "$shlibs"
if [ -n "$found" ]; then
    depends=$found
else
    # Rather than write a Depends nobody checked: a package that installs and
    # then cannot start is worse than one that did not build.
    echo "dpkg-shlibdeps could not name this binary's dependencies" >&2
    exit 1
fi

installed_kb=$(du -sk "$staging" | cut -f1)
mkdir -p "$staging/DEBIAN"
cat > "$staging/DEBIAN/control" <<CONTROL
Package: uniwidged
Version: $version
Section: x11
Priority: optional
Architecture: $arch
Depends: $depends
Installed-Size: $installed_kb
Maintainer: uniwidged contributors <noreply@example.invalid>
Homepage: https://github.com/j-pfalzgraf/uniwidged
Description: Cross-platform desktop widget framework
 One host process owns one GPU device and every widget's surface. Widgets sit
 below your windows and above your wallpaper, composited per-pixel against it.
 They are WASM components: sandboxed, installable at runtime, and written in
 any language that targets the component model.
 .
 This package installs the uniwidged binary, which is the host and the
 command-line tool both. Widgets are installed per user with
 "uniwidged install" or from a registry with "uniwidged add".
CONTROL

mkdir -p "$output"
package="$output/uniwidged_${version}_${arch}.deb"
dpkg-deb --build --root-owner-group "$staging" "$package" >/dev/null
echo "wrote $package"
