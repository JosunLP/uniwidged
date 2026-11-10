#!/bin/sh
# Builds uniwidged.app — and, when it can, signs and notarises it.
#
#   packaging/macos/build.sh [--output DIR] [--target-dir DIR] [--universal]
#
# The bundle is a binary, an Info.plist and an icon, assembled with `cp` and
# `plutil` and nothing else, for the reason the .deb is built with `dpkg-deb`:
# every file in it can be checked by reading.
#
# `LSUIElement` is set, so the bundle has no Dock tile and no menu bar even
# before the binary's own `.accessory` policy takes effect — the two agree, and
# a bundle that flashed a Dock icon at launch would be a widget host that
# announces itself.
#
# Signing and notarisation happen only when they are asked for:
#
#   MACOS_SIGNING_IDENTITY   "Developer ID Application: …" — sign with the
#                            hardened runtime, as notarisation requires
#   MACOS_NOTARY_PROFILE     a `notarytool store-credentials` profile name —
#                            submit, wait, and staple the ticket
#
# Without an identity the bundle is signed ad hoc (`codesign -s -`), which an
# Apple Silicon Mac requires before it will run anything at all, and the script
# says that the result is not notarised. Gatekeeper will refuse to open it from
# a download; `xattr -d com.apple.quarantine` is the honest workaround, and the
# packaging README says so.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
output="$root/target/macos"
target_dir="$root/target"
universal=0

while [ $# -gt 0 ]; do
    case "$1" in
        --output) output=$2; shift 2 ;;
        --target-dir) target_dir=$2; shift 2 ;;
        --universal) universal=1; shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

need() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "$1 is needed to build the .app and is not on this machine" >&2
        exit 1
    }
}
need cargo
need codesign
need plutil
need ditto

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }

echo "building uniwidged $version"
if [ "$universal" = 1 ]; then
    need lipo
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
        cargo build --release --locked --package uniwidged-cli \
            --target "$target" --target-dir "$target_dir"
    done
    binary="$target_dir/universal/uniwidged"
    mkdir -p "$(dirname "$binary")"
    lipo -create -output "$binary" \
        "$target_dir/aarch64-apple-darwin/release/uniwidged" \
        "$target_dir/x86_64-apple-darwin/release/uniwidged"
else
    cargo build --release --locked --package uniwidged-cli --target-dir "$target_dir"
    binary="$target_dir/release/uniwidged"
fi
[ -x "$binary" ] || { echo "$binary was not built" >&2; exit 1; }

app="$output/uniwidged.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/uniwidged"
cp "$root/assets/logo-512.png" "$app/Contents/Resources/uniwidged.png"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>io.github.josunlp.uniwidged</string>
	<key>CFBundleName</key>
	<string>uniwidged</string>
	<key>CFBundleDisplayName</key>
	<string>uniwidged</string>
	<key>CFBundleExecutable</key>
	<string>uniwidged</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$version</string>
	<key>CFBundleVersion</key>
	<string>$version</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>LSUIElement</key>
	<true/>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSHumanReadableCopyright</key>
	<string>MIT OR Apache-2.0</string>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist" >/dev/null

if [ -n "${MACOS_SIGNING_IDENTITY:-}" ]; then
    # The hardened runtime, and a timestamp: both are what notarisation checks
    # for. The entitlements are the two Wasmtime's compiled code needs — see
    # the file for why, and why nothing else is in it.
    codesign --force --options runtime --timestamp \
        --entitlements "$root/packaging/macos/uniwidged.entitlements" \
        --sign "$MACOS_SIGNING_IDENTITY" "$app"
    signed=1
else
    codesign --force --sign - "$app"
    signed=0
fi
codesign --verify --strict "$app"

zip="$output/uniwidged-$version-macos.zip"
rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"

if [ "$signed" = 1 ] && [ -n "${MACOS_NOTARY_PROFILE:-}" ]; then
    need xcrun
    xcrun notarytool submit "$zip" --keychain-profile "$MACOS_NOTARY_PROFILE" --wait
    xcrun stapler staple "$app"
    # The zip was made before the ticket existed; make it again so the download
    # carries the stapled bundle.
    rm -f "$zip"
    ditto -c -k --keepParent "$app" "$zip"
    echo "signed and notarised: $zip"
elif [ "$signed" = 1 ]; then
    echo "signed, NOT notarised (set MACOS_NOTARY_PROFILE): $zip"
else
    echo "ad-hoc signed, NOT notarised (set MACOS_SIGNING_IDENTITY): $zip"
fi
