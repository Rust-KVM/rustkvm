#!/usr/bin/env bash
# Package the corresponding source for a RustKVM release (GPL-2.0 compliance).
#
#   scripts/gpl-source-bundle.sh                 → out/release/rustkvm-<version>-source.tar.gz
#   OUT=dir scripts/gpl-source-bundle.sh
#   LEGAL_INFO=1 scripts/gpl-source-bundle.sh    also run Buildroot `make legal-info` in the
#                                                firmware SDK and ship it as a second archive
#
# The first archive holds the RustKVM tree at this commit (application + firmware
# layer: pinned SDK manifest, patches, rootfs overlay) and SOURCES.md, which lists
# every upstream SDK project at its exact commit. The legal-info archive adds the
# license texts and source tarballs of every package in the root filesystem.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${OUT:-$ROOT/out/release}
SDK=${RUSTKVM_SDK_DIR:-$HOME/rustkvm-sdk/rk3588}
version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)
rev=$(git -C "$ROOT" rev-parse HEAD)
name=rustkvm-$version-source
mkdir -p "$OUT"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

git -C "$ROOT" archive --format=tar --prefix="$name/" HEAD | tar -x -C "$stage"

{
    echo "# RustKVM $version: corresponding source"
    echo
    echo "RustKVM tree: https://github.com/Rust-KVM/rustkvm/tree/$rev"
    echo
    echo "The firmware is the FriendlyElec RK3588 Buildroot SDK at the commits below,"
    echo "with os/rk3588/patches applied and os/rk3588/rootfs-overlay installed."
    echo "scripts/build-firmware.sh reproduces it."
    echo
    echo "| Path | Repository | Commit |"
    echo "| --- | --- | --- |"
    sed -n 's|.*path="\([^"]*\)" name="\([^"]*\)" remote="friendlyarm" revision="\([0-9a-f]*\)".*|\| \1 \| https://github.com/friendlyarm/\2 \| \3 \||p' \
        "$ROOT/os/rk3588/manifest.xml"
    echo
    echo "Source for any GPL component is also available on request for three years"
    echo "from the date of the release, via https://github.com/Rust-KVM/rustkvm/issues."
} >"$stage/$name/SOURCES.md"

tar -C "$stage" -czf "$OUT/$name.tar.gz" "$name"
echo "==> $OUT/$name.tar.gz"

if [ "${LEGAL_INFO:-0}" = 1 ]; then
    out_dir=$SDK/buildroot/output/rockchip_rk3588
    [ -f "$out_dir/.config" ] || { echo "error: no built Buildroot tree in $out_dir" >&2; exit 1; }
    make -C "$SDK/buildroot" O="$out_dir" legal-info
    tar -C "$out_dir" -czf "$OUT/rustkvm-$version-legal-info.tar.gz" legal-info
    echo "==> $OUT/rustkvm-$version-legal-info.tar.gz"
fi
