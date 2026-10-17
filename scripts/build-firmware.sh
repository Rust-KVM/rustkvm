#!/usr/bin/env bash
# Build the RustKVM RK3588 firmware image from public sources.
#
#   scripts/build-firmware.sh [STEP...]
#
# Steps (default: all, in this order):
#   sync    check out the FriendlyElec Buildroot SDK at the commits pinned in
#           os/rk3588/manifest.xml (needs `repo`)
#   patch   reset patched projects, apply os/rk3588/patches, install the rootfs
#           overlay, the application binary and the release file
#   build   kernel + u-boot + Buildroot + SD image + eMMC (eflasher) image
#   collect copy versioned images and SHA256SUMS into out/firmware/
#
# Environment:
#   RUSTKVM_SDK_DIR  SDK checkout (default ~/rustkvm-sdk/rk3588). Created and owned by
#                    this script; never point it at a hand-maintained SDK, because
#                    `patch` resets the patched projects.
#   RUSTKVM_APP      rustkvm_app binary to install (default out/app/rustkvm_app-*)
#   BOARD_MK         FriendlyElec board config (default rk3588.mk)
#   RUSTKVM_ROOT_PASSWORD
#                    root password for the serial console, preferably a crypt hash
#                    (mkpasswd -m sha-512). Unset: root password login is disabled.
#                    SSH is key-only either way (devmode, dropbear -s).
#   REPO_DEPTH       clone depth for `repo sync` (default 1; 0 = full history)
#   BR2_DL_DIR       Buildroot download cache (honoured by Buildroot itself)
#
# Easiest from any Linux host with Docker: scripts/devkit.sh firmware
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OS=$ROOT/os/rk3588
SDK=${RUSTKVM_SDK_DIR:-$HOME/rustkvm-sdk/rk3588}
APP=${RUSTKVM_APP:-$ROOT/out/app/rustkvm_app-aarch64-unknown-linux-gnu}
BOARD_MK=${BOARD_MK:-rk3588.mk}
REPO_DEPTH=${REPO_DEPTH:-1}
OUT=$ROOT/out/firmware
MARKER=.rustkvm-sdk

app_version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)
rev=$(git -C "$ROOT" rev-parse --short=8 HEAD 2>/dev/null || echo unknown)
FW_VERSION=${FW_VERSION:-$app_version-$(date -u +%Y%m%d)-$rev}

log() { printf '\033[1;32m==> %s\033[0m\n' "$*"; }
die() { printf '\033[1;31merror: %s\033[0m\n' "$*" >&2; exit 1; }

guard_sdk() {
    mkdir -p "$SDK"
    if [ -n "$(ls -A "$SDK")" ] && [ ! -e "$SDK/$MARKER" ]; then
        die "$SDK is not empty and was not created by this script; use another RUSTKVM_SDK_DIR"
    fi
    touch "$SDK/$MARKER"
}

patched_projects() {
    (cd "$OS/patches" && find . -name '*.patch' -printf '%h\n' | sed 's|^\./||' | sort -u)
}

step_sync() {
    command -v repo >/dev/null || die "repo not found (https://gerrit.googlesource.com/git-repo)"
    guard_sdk
    log "sync SDK into $SDK"
    # repo wants a manifest git repository; serve the pinned manifest from a local one.
    local mf=$SDK/.rustkvm-manifest
    rm -rf "$mf"
    git init -q -b rustkvm "$mf"
    cp "$OS/manifest.xml" "$mf/default.xml"
    git -C "$mf" add default.xml
    git -C "$mf" -c user.name=rustkvm -c user.email=build@localhost -c commit.gpgsign=false \
        commit -q -m "pinned manifest"

    # Discard earlier patches so repo can move projects to the pinned commits.
    if [ -d "$SDK/.repo" ]; then
        while read -r p; do
            [ -d "$SDK/$p/.git" ] || continue
            git -C "$SDK/$p" reset -q --hard
            git -C "$SDK/$p" clean -fdq
        done < <(patched_projects)
    fi

    cd "$SDK"
    local depth=()
    [ "$REPO_DEPTH" -gt 0 ] && depth=(--depth="$REPO_DEPTH")
    repo init -q -u "file://$mf" -b rustkvm -m default.xml "${depth[@]}"
    repo sync -q -c -d --no-tags --force-sync -j"$(nproc)"
}

step_patch() {
    [ -e "$SDK/$MARKER" ] || die "$SDK has no SDK checkout; run the sync step first"
    log "apply patches"
    while read -r p; do
        git -C "$SDK/$p" reset -q --hard
        git -C "$SDK/$p" clean -fdq
        for f in "$OS/patches/$p"/*.patch; do
            echo "    $p: $(basename "$f")"
            git -C "$SDK/$p" apply --whitespace=nowarn "$f"
        done
    done < <(patched_projects)

    local defconfig=$SDK/buildroot/configs/rockchip_rk3588_defconfig
    sed -i '/^BR2_TARGET_GENERIC_ROOT_PASSWD=/d;/BR2_TARGET_ENABLE_ROOT_LOGIN/d' "$defconfig"
    if [ -n "${RUSTKVM_ROOT_PASSWORD:-}" ]; then
        log "root password login enabled"
        # Buildroot reads the value through make: a crypt hash needs its '$' doubled.
        printf 'BR2_TARGET_GENERIC_ROOT_PASSWD="%s"\n' "${RUSTKVM_ROOT_PASSWORD//\$/\$\$}" >>"$defconfig"
    else
        log "root password login disabled (set RUSTKVM_ROOT_PASSWORD to enable)"
        echo '# BR2_TARGET_ENABLE_ROOT_LOGIN is not set' >>"$defconfig"
    fi

    log "install rootfs overlay ($FW_VERSION)"
    local files=$SDK/device/friendlyelec/rustkvm-files
    rm -rf "$files"
    cp -a "$OS/rootfs-overlay" "$files"
    cat >"$files/etc/rustkvm-release" <<EOF
FIRMWARE_VERSION=$FW_VERSION
APP_VERSION=$app_version
GIT_REVISION=$rev
BOARD=${BOARD_MK%.mk}
EOF

    [ -f "$APP" ] || die "application binary not found: $APP (run scripts/build-app.sh first)"
    log "install application $(basename "$APP")"
    local bin=$SDK/device/rockchip/common/images/userdata/normal/rustkvm/bin
    mkdir -p "$bin"
    install -m 0755 "$APP" "$bin/rustkvm_app"
}

step_build() {
    cd "$SDK"
    log "build kernel, u-boot, Buildroot and SD image ($BOARD_MK)"
    ./build.sh "$BOARD_MK"
    log "build eMMC flasher image"
    ./build.sh emmc-img
}

step_collect() {
    log "collect images into $OUT"
    mkdir -p "$OUT"
    local sd eflasher
    sd=$(ls -t "$SDK"/out/*_sd.img 2>/dev/null | head -n1) || true
    eflasher=$(ls -t "$SDK"/out/*_eflasher.img 2>/dev/null | head -n1) || true
    [ -n "$sd" ] || die "no SD image under $SDK/out"
    local name=rustkvm-rk3588-$FW_VERSION
    gzip -c "$sd" >"$OUT/$name-sd.img.gz"
    [ -n "$eflasher" ] && gzip -c "$eflasher" >"$OUT/$name-eflasher.img.gz"
    cp "$SDK/device/friendlyelec/rustkvm-files/etc/rustkvm-release" "$OUT/$name.release"
    (cd "$OUT" && sha256sum "$name"-* "$name.release" >"$name.SHA256SUMS")
    ls -lh "$OUT"/"$name"*
}

steps=("$@")
[ ${#steps[@]} -eq 0 ] && steps=(sync patch build collect)
[ "${steps[0]}" = all ] && steps=(sync patch build collect)
for s in "${steps[@]}"; do
    case "$s" in
        sync | patch | build | collect) "step_$s" ;;
        *) die "unknown step: $s (sync, patch, build, collect, all)" ;;
    esac
done
