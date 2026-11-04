#!/usr/bin/env bash
# Builds WEEP for Linux and packs the release tarball.
#
#   tools/linux/package.sh [--skip-build] [--out-dir dist]
#
# Runs `cargo build --release --locked -p weep` (into $CARGO_TARGET_DIR when set),
# checks the binary is what a release needs, then writes
# <out-dir>/weep-<version>-linux-<arch>.tar.gz and its .sha256:
#
#   weep-<version>-linux-<arch>/
#     weep                  icons, fonts, themes and keymap are compiled in
#     weep.desktop          the menu entry, for ~/.local/share/applications
#     weep.png              its icon, for ~/.local/share/icons/hicolor/256x256/apps
#     LICENSE.txt           WEEP (MIT)
#     licenses/             what the binary bundles: Inter and JetBrains Mono (OFL),
#                           JetBrains' expui icons (Apache-2.0)
#
# The checks: a dynamically linked ELF executable with every library resolved, and
# the glibc the build needs, printed so the floor is on the record. gpui links libxcb
# and libxkbcommon, and dlopens wayland, libvulkan and libGL at run time; git, the
# shells and rust-analyzer are found on PATH.
#
# Build it on the oldest glibc you mean to support (the release workflow uses Ubuntu
# 22.04): the tarball runs on that glibc and newer, not older.
set -euo pipefail

skip_build=false
out_dir=dist
while [ $# -gt 0 ]; do
    case "$1" in
        --skip-build) skip_build=true ;;
        --out-dir) out_dir="$2"; shift ;;
        *) echo "usage: $0 [--skip-build] [--out-dir dist]" >&2; exit 2 ;;
    esac
    shift
done

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo"

pkgid="$(cargo pkgid -p weep)"            # path+file:///...#weep@0.1.0
version="${pkgid##*[@#]}"
target_dir="${CARGO_TARGET_DIR:-$repo/target}"
case "$(uname -m)" in
    x86_64) arch=x64 ;;
    aarch64|arm64) arch=arm64 ;;
    *) arch="$(uname -m)" ;;
esac

$skip_build || cargo build --release --locked -p weep
bin="$target_dir/release/weep"
[ -f "$bin" ] || { echo "no $bin; build first or drop --skip-build" >&2; exit 1; }

# The release checks.
head -c 4 "$bin" | grep -q $'\x7fELF' || { echo "$bin is not an ELF file" >&2; exit 1; }
if missing="$(ldd "$bin" | grep 'not found' || true)"; [ -n "$missing" ]; then
    echo "unresolved libraries:" >&2; echo "$missing" >&2; exit 1
fi
# Crude on purpose: the versioned glibc symbols are in the binary's string table, so
# the floor needs no binutils to read.
glibc="$(grep -ao 'GLIBC_[0-9]\+\.[0-9]\+' "$bin" | sort -V -u | tail -1)"

name="weep-$version-linux-$arch"
stage="$out_dir/$name"
rm -rf "$stage"
mkdir -p "$stage/licenses"

install -m 755 "$bin" "$stage/weep"
install -m 644 crates/weep/assets/icons/weep/weep.png "$stage/weep.png"
cat > "$stage/weep.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=WEEP
GenericName=Code Editor
Comment=A native code editor in the shape of JetBrains Fleet
Exec=weep %F
Icon=weep
Terminal=false
Categories=Development;IDE;TextEditor;
MimeType=text/plain;text/x-rust;application/json;text/x-toml;text/markdown;inode/directory;
StartupWMClass=weep
DESKTOP

install -m 644 LICENSE "$stage/LICENSE.txt"
assets=crates/weep/assets
install -m 644 "$assets/fonts/LICENSE-Inter.txt" "$stage/licenses/Inter-OFL.txt"
install -m 644 "$assets/fonts/LICENSE-JetBrainsMono.txt" "$stage/licenses/JetBrainsMono-OFL.txt"
install -m 644 "$assets/icons/expui/LICENSE.txt" "$stage/licenses/expui-icons-NOTICE.txt"
install -m 644 "$assets/icons/expui/LICENSE-Apache-2.0.txt" "$stage/licenses/Apache-2.0.txt"

tar="$out_dir/$name.tar.gz"
rm -f "$tar" "$tar.sha256"
tar -czf "$tar" -C "$out_dir" "$name"
hash="$(sha256sum "$tar" | cut -d' ' -f1)"
printf '%s  %s.tar.gz' "$hash" "$name" > "$tar.sha256"

echo "$(cd "$(dirname "$tar")" && pwd)/$(basename "$tar")  ($(du -m "$tar" | cut -f1) MB, needs $glibc)"
echo "sha256 $hash"
