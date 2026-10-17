#!/usr/bin/env bash
# Build the RustKVM application package: frontend (Trunk) + device binary.
#
#   scripts/build-app.sh            # release build into out/app/
#   scripts/devkit.sh run scripts/build-app.sh   # same, inside the devkit
#
# Works on the reference workbench (rust-toolchain.toml → stage2) and in the
# devkit container (RUSTUP_TOOLCHAIN → pinned nightly).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${OUT:-$ROOT/out/app}
TARGET=aarch64-unknown-linux-gnu
ASSET=rustkvm_app-$TARGET

cd "$ROOT"

echo "==> frontend (trunk build --release --locked)"
(
    cd crates/rkvm-web
    if [ -n "${RKVM_WEB_TOOLCHAIN:-}" ]; then
        export RUSTUP_TOOLCHAIN=$RKVM_WEB_TOOLCHAIN
    else
        unset RUSTUP_TOOLCHAIN
    fi
    trunk build --release --locked
)

echo "==> device binary ($TARGET)"
cargo build -Z build-std=std,panic_abort --locked --release \
    --target "$TARGET" -p rustkvm --bin rustkvm_app

version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n1)
rev=$(git rev-parse --short=12 HEAD 2>/dev/null || echo unknown)
dirty=$(git status --porcelain --untracked-files=no 2>/dev/null | grep -q . && echo "-dirty" || true)

mkdir -p "$OUT"
cp "target/$TARGET/release/rustkvm_app" "$OUT/$ASSET"
(cd "$OUT" && sha256sum "$ASSET" >"$ASSET.sha256")
cat >"$OUT/BUILD_INFO" <<EOF
version=$version
git_revision=$rev$dirty
build_date=$(date -u +%Y-%m-%dT%H:%M:%SZ)
rustc=$(rustc -V)
EOF

echo "==> done: $OUT"
cat "$OUT/BUILD_INFO"
