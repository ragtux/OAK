#!/usr/bin/env bash
# Builds WEEP for macOS and packs the release zip.
#
#   tools/macos/package.sh [--native] [--skip-build] [--out-dir dist]
#
# Runs `cargo build --release --locked -p weep` for both Apple targets and joins them
# with `lipo` (--native builds only the machine's own arch), stages WEEP.app around the
# binary, checks it, then writes <out-dir>/weep-<version>-macos-<arch>.zip and its
# .sha256:
#
#   weep-<version>-macos-<arch>/
#     WEEP.app/                 Contents/MacOS/weep, the icon and Info.plist; the
#                               fonts, themes and keymap are compiled in
#     LICENSE.txt               WEEP (MIT)
#     licenses/                 what the binary bundles: Inter and JetBrains Mono
#                               (OFL), JetBrains' expui icons (Apache-2.0)
#
# The checks: a Mach-O binary with the architectures asked for, an ad-hoc signature
# (arm64 refuses to run unsigned, and `lipo` drops the one cargo makes), and an
# Info.plist `plutil` accepts. git, the shells and rust-analyzer are found on PATH at
# run time.
#
# The zip is not notarized, so the first launch needs Finder's right-click > Open, or
# `xattr -dr com.apple.quarantine /Applications/WEEP.app`.
set -euo pipefail

skip_build=false
out_dir=dist
archs=(x86_64-apple-darwin aarch64-apple-darwin)
while [ $# -gt 0 ]; do
    case "$1" in
        --native) archs=("$(rustc -vV | sed -n 's/^host: //p')") ;;
        --skip-build) skip_build=true ;;
        --out-dir) out_dir="$2"; shift ;;
        *) echo "usage: $0 [--native] [--skip-build] [--out-dir dist]" >&2; exit 2 ;;
    esac
    shift
done

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo"
[ "$(uname -s)" = Darwin ] || { echo "$0 runs on macOS" >&2; exit 1; }

pkgid="$(cargo pkgid -p weep)"            # path+file:///...#weep@0.1.0
version="${pkgid##*[@#]}"
target_dir="${CARGO_TARGET_DIR:-$repo/target}"
# The oldest macOS WEEP is built for; gpui needs 11 for its Metal path.
export MACOSX_DEPLOYMENT_TARGET=11.0

binaries=()
for triple in "${archs[@]}"; do
    if ! $skip_build; then
        rustup target add "$triple" >/dev/null
        cargo build --release --locked -p weep --target "$triple"
    fi
    bin="$target_dir/$triple/release/weep"
    [ -f "$bin" ] || { echo "no $bin; build first or drop --skip-build" >&2; exit 1; }
    binaries+=("$bin")
done

if [ "${#binaries[@]}" -gt 1 ]; then
    name_arch=universal
else
    case "${archs[0]}" in
        aarch64-*) name_arch=arm64 ;;
        x86_64-*) name_arch=x64 ;;
        *) name_arch="${archs[0]%%-*}" ;;
    esac
fi

name="weep-$version-macos-$name_arch"
stage="$out_dir/$name"
app="$stage/WEEP.app"
rm -rf "$stage"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$stage/licenses"

lipo -create -output "$app/Contents/MacOS/weep" "${binaries[@]}"
chmod +x "$app/Contents/MacOS/weep"
cp crates/weep/assets/icons/weep/weep.icns "$app/Contents/Resources/weep.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>WEEP</string>
    <key>CFBundleDisplayName</key><string>WEEP</string>
    <key>CFBundleIdentifier</key><string>cc.weep.weep</string>
    <key>CFBundleExecutable</key><string>weep</string>
    <key>CFBundleIconFile</key><string>weep</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>$MACOSX_DEPLOYMENT_TARGET</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>Copyright (c) 2026 ragtux. MIT License.</string>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Text</string>
            <key>CFBundleTypeRole</key><string>Editor</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>LSItemContentTypes</key>
            <array>
                <string>public.plain-text</string>
                <string>public.source-code</string>
            </array>
        </dict>
        <dict>
            <key>CFBundleTypeName</key><string>Folder</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>LSItemContentTypes</key><array><string>public.folder</string></array>
        </dict>
    </array>
</dict>
</plist>
PLIST

cp LICENSE "$stage/LICENSE.txt"
assets=crates/weep/assets
cp "$assets/fonts/LICENSE-Inter.txt" "$stage/licenses/Inter-OFL.txt"
cp "$assets/fonts/LICENSE-JetBrainsMono.txt" "$stage/licenses/JetBrainsMono-OFL.txt"
cp "$assets/icons/expui/LICENSE.txt" "$stage/licenses/expui-icons-NOTICE.txt"
cp "$assets/icons/expui/LICENSE-Apache-2.0.txt" "$stage/licenses/Apache-2.0.txt"

# Ad-hoc signature, over the finished bundle: `lipo` writes an unsigned binary, and
# macOS kills an unsigned arm64 process on launch.
codesign --force --sign - --timestamp=none "$app"

plutil -lint "$app/Contents/Info.plist" >/dev/null
codesign --verify --strict "$app"
got="$(lipo -archs "$app/Contents/MacOS/weep")"
for triple in "${archs[@]}"; do
    want="${triple%%-*}"; want="${want/aarch64/arm64}"
    case " $got " in *" $want "*) ;; *) echo "binary has [$got], not $want" >&2; exit 1 ;; esac
done

zip="$out_dir/$name.zip"
rm -f "$zip" "$zip.sha256"
ditto -c -k --sequesterRsrc --keepParent "$stage" "$zip"
hash="$(shasum -a 256 "$zip" | cut -d' ' -f1)"
printf '%s  %s.zip' "$hash" "$name" > "$zip.sha256"

echo "$(cd "$(dirname "$zip")" && pwd)/$(basename "$zip")  ($(du -m "$zip" | cut -f1) MB, $got)"
echo "sha256 $hash"
