#!/usr/bin/env bash
# Containerised build environment for RustKVM. Only Docker is required on the host.
#
#   scripts/devkit.sh buildkit [DIR]   build the RK3588 toolchain image from a Buildroot
#                                      host dir (default /opt/rk3588-buildkit)
#   scripts/devkit.sh image [--if-missing]
#                                      build the devkit image (frontend + device binary);
#                                      --if-missing pulls the published one first
#   scripts/devkit.sh push             publish the buildkit and devkit images (maintainers)
#   scripts/devkit.sh shell            interactive shell in the devkit, repo at /work
#   scripts/devkit.sh run CMD...       run CMD in the devkit, e.g. scripts/build-app.sh
#   scripts/devkit.sh firmware-image   build the Buildroot SDK host image
#   scripts/devkit.sh firmware ARGS... run scripts/build-firmware.sh ARGS in that image
#
# Images (published copies are pulled; override to use your own):
#   RUSTKVM_BUILDKIT_IMAGE  default ghcr.io/rust-kvm/rk3588-buildkit:20250527
#   RUSTKVM_DEVKIT_IMAGE    default ghcr.io/rust-kvm/rustkvm-devkit:nightly-2026-06-14
#   RUSTKVM_FIRMWARE_IMAGE  default rustkvm/firmware-builder:local
# Caches (cargo registry, trunk tools, Buildroot downloads, ccache):
#   RUSTKVM_CACHE_DIR       default ~/.cache/rustkvm
#   RUSTKVM_SDK_DIR         default ~/rustkvm-sdk/rk3588 (firmware SDK checkout, ~60 GB)
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
# Bump the tags together with the SDK snapshot / RUST_NIGHTLY in ci/devkit/Dockerfile.
BUILDKIT_IMAGE=${RUSTKVM_BUILDKIT_IMAGE:-ghcr.io/rust-kvm/rk3588-buildkit:20250527}
DEVKIT_IMAGE=${RUSTKVM_DEVKIT_IMAGE:-ghcr.io/rust-kvm/rustkvm-devkit:nightly-2026-06-14}
FIRMWARE_IMAGE=${RUSTKVM_FIRMWARE_IMAGE:-rustkvm/firmware-builder:local}
CACHE=${RUSTKVM_CACHE_DIR:-$HOME/.cache/rustkvm}
SDK=${RUSTKVM_SDK_DIR:-$HOME/rustkvm-sdk/rk3588}

have_image() { docker image inspect "$1" >/dev/null 2>&1; }

TTY=(-i)
if [ -t 0 ] && [ -t 1 ]; then TTY=(-it); fi

# Forward the host's proxy settings into builds and containers (host network,
# so a proxy on 127.0.0.1 keeps working).
BUILD_PROXY=(--network host)
RUN_PROXY=()
for v in http_proxy https_proxy no_proxy HTTP_PROXY HTTPS_PROXY NO_PROXY; do
    if [ -n "${!v:-}" ]; then
        BUILD_PROXY+=(--build-arg "$v")
        RUN_PROXY+=(-e "$v")
    fi
done

cmd_buildkit() {
    local dir=${1:-/opt/rk3588-buildkit}
    if [ ! -x "$dir/bin/aarch64-buildroot-linux-gnu-gcc" ]; then
        echo "error: $dir is not a Buildroot host dir (no bin/aarch64-buildroot-linux-gnu-gcc)" >&2
        echo "Build the firmware once (scripts/devkit.sh firmware) and pass" >&2
        echo "  \$RUSTKVM_SDK_DIR/buildroot/output/rockchip_rk3588/host" >&2
        exit 1
    fi
    docker buildx build --load "${BUILD_PROXY[@]}" --build-context "rk3588-buildkit=$dir" \
        -t "$BUILDKIT_IMAGE" "$ROOT/ci/buildkit"
}

cmd_image() {
    if [ "${1:-}" = "--if-missing" ]; then
        have_image "$DEVKIT_IMAGE" && return 0
        docker pull "$DEVKIT_IMAGE" && return 0
        echo "==> $DEVKIT_IMAGE not available; building it locally"
    fi
    if ! have_image "$BUILDKIT_IMAGE" && ! docker pull "$BUILDKIT_IMAGE" 2>/dev/null; then
        echo "==> $BUILDKIT_IMAGE not found; building it from /opt/rk3588-buildkit"
        cmd_buildkit
    fi
    docker build "${BUILD_PROXY[@]}" --build-arg "BUILDKIT_IMAGE=$BUILDKIT_IMAGE" \
        -t "$DEVKIT_IMAGE" "$ROOT/ci/devkit"
}

devkit_run() {
    cmd_image --if-missing
    mkdir -p "$CACHE/home" "$CACHE/cargo"
    docker run --rm "${TTY[@]}" \
        --user "$(id -u):$(id -g)" \
        -e HOME=/cache/home -e CARGO_HOME=/cache/cargo \
        -v "$ROOT:/work" -v "$CACHE:/cache" -w /work \
        --network host "${RUN_PROXY[@]}" \
        "$DEVKIT_IMAGE" "$@"
}

cmd_push() {
    have_image "$BUILDKIT_IMAGE" || cmd_buildkit
    have_image "$DEVKIT_IMAGE" || cmd_image
    docker push "$BUILDKIT_IMAGE"
    docker push "$DEVKIT_IMAGE"
}

cmd_firmware_image() {
    docker build "${BUILD_PROXY[@]}" --build-arg "UID=$(id -u)" --build-arg "GID=$(id -g)" \
        -t "$FIRMWARE_IMAGE" "$ROOT/ci/firmware"
}

cmd_firmware() {
    have_image "$FIRMWARE_IMAGE" || cmd_firmware_image
    mkdir -p "$SDK" "$CACHE/dl" "$CACHE/ccache"
    # --privileged: the SDK packs SD/eMMC images through loop devices.
    docker run --rm "${TTY[@]}" --privileged \
        -e RUSTKVM_SDK_DIR=/sdk -e BR2_DL_DIR=/cache/dl \
        -e RUSTKVM_ROOT_PASSWORD -e FW_VERSION -e BOARD_MK -e REPO_DEPTH \
        -v "$ROOT:/work" -v "$SDK:/sdk" -v "$CACHE:/cache" \
        -v "$CACHE/ccache:/home/builder/.buildroot-ccache" \
        -w /work --network host "${RUN_PROXY[@]}" \
        "$FIRMWARE_IMAGE" scripts/build-firmware.sh "$@"
}

case "${1:-}" in
    buildkit) shift; cmd_buildkit "$@" ;;
    image) shift; cmd_image "$@" ;;
    push) cmd_push ;;
    shell) devkit_run bash ;;
    run) shift; devkit_run "$@" ;;
    firmware-image) cmd_firmware_image ;;
    firmware) shift; cmd_firmware "$@" ;;
    *) sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//'; exit 1 ;;
esac
