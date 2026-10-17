# RustKVM devkit: everything needed to build the frontend and the device
# binary, on top of the RK3588 buildkit image (cross gcc + sysroot).
#
#   scripts/devkit.sh image      # build it locally
#   scripts/devkit.sh shell      # use it
#
# Toolchain versions are pinned here and nowhere else in the container path.
ARG BUILDKIT_IMAGE=ghcr.io/rust-kvm/rk3588-buildkit:20250527
FROM ${BUILDKIT_IMAGE}

# One pinned nightly builds both the device binary (-Z build-std) and the
# frontend (wasm32). Matches what release.yml uses.
ARG RUST_NIGHTLY=nightly-2026-06-14
ARG TRUNK_VERSION=0.21.14

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
        openssh-client rsync sudo \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:/opt/rk3588-buildkit/bin:$PATH

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --no-modify-path --profile minimal --default-toolchain none \
    && rustup toolchain install "${RUST_NIGHTLY}" --profile minimal \
        --component rust-src,rustfmt,clippy \
        --target aarch64-unknown-linux-gnu,wasm32-unknown-unknown \
    && rustup default "${RUST_NIGHTLY}" \
    && curl -sSfL "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-x86_64-unknown-linux-gnu.tar.gz" \
        | tar -xz -C /usr/local/cargo/bin trunk \
    && chmod -R a+w "${RUSTUP_HOME}" "${CARGO_HOME}"

# RUSTUP_TOOLCHAIN overrides the repo's rust-toolchain.toml (`stage2`, a
# toolchain only the reference workbench has), and crates/rkvm-web's `stable`.
ENV RUSTUP_TOOLCHAIN=${RUST_NIGHTLY} \
    RKVM_WEB_TOOLCHAIN=${RUST_NIGHTLY} \
    CC_aarch64_unknown_linux_gnu=/opt/rk3588-buildkit/bin/aarch64-buildroot-linux-gnu-gcc

# build.rs reads git metadata from a bind-mounted checkout owned by another uid.
# The devcontainer runs as `ubuntu` (uid remapped to the host user).
RUN git config --system --add safe.directory '*' \
    && (id ubuntu >/dev/null 2>&1 || useradd -m -s /bin/bash -u 1000 ubuntu)

LABEL org.opencontainers.image.source="https://github.com/Rust-KVM/rustkvm" \
      org.opencontainers.image.description="RustKVM devkit: pinned Rust nightly, trunk and the RK3588 cross toolchain" \
      org.opencontainers.image.licenses="GPL-2.0-only"
