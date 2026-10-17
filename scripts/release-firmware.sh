#!/usr/bin/env bash
# Attach a locally built firmware to the DRAFT GitHub release of the same version.
#
#   scripts/release-firmware.sh [VERSION]      default: workspace version
#
# Expects out/firmware/ from `scripts/devkit.sh firmware` (FW_VERSION=<version>-...).
# Signs the firmware checksum file with GPG (RELEASE_GPG_KEY selects the key,
# default gpg's default key), optionally builds the Buildroot legal-info bundle
# (LEGAL_INFO=1), and uploads everything with gh. It refuses to touch a release
# that is already published.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
version=${1:-$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)}
tag=v$version
FW=$ROOT/out/firmware

draft=$(gh release view "$tag" --json isDraft --jq .isDraft 2>/dev/null) || {
    echo "error: no release $tag; run the Release workflow first" >&2
    exit 1
}
[ "$draft" = true ] || { echo "error: $tag is already published" >&2; exit 1; }

sums=$(ls -t "$FW"/rustkvm-rk3588-"$version"-*.SHA256SUMS 2>/dev/null | head -n1) || true
[ -n "$sums" ] || { echo "error: no firmware for $version in $FW" >&2; exit 1; }
base=${sums%.SHA256SUMS}

key=()
[ -n "${RELEASE_GPG_KEY:-}" ] && key=(--local-user "$RELEASE_GPG_KEY")
gpg --batch --yes "${key[@]}" --armor --detach-sign --output "$sums.asc" "$sums"
gpg --batch --verify "$sums.asc" "$sums"

files=("$base"-*.img.gz "$base.release" "$sums" "$sums.asc")
if [ "${LEGAL_INFO:-0}" = 1 ]; then
    OUT=$FW LEGAL_INFO=1 "$ROOT/scripts/gpl-source-bundle.sh"
    files+=("$FW/rustkvm-$version-legal-info.tar.gz")
fi

ls -lh "${files[@]}"
gh release upload "$tag" --clobber "${files[@]}"
echo "==> attached to draft $tag; review it on GitHub before publishing"
