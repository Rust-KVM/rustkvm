# RK3588 cross toolchain + sysroot (the Buildroot `host/` directory) at the
# path .cargo/config.toml expects. The directory comes in as the named build
# context `rk3588-buildkit`:
#
#   docker buildx build --build-context rk3588-buildkit=/opt/rk3588-buildkit \
#     -t ghcr.io/rust-kvm/rk3588-buildkit:20250527 ci/buildkit
#
# scripts/devkit.sh buildkit does exactly this.
FROM ubuntu:24.04 AS prune

COPY --from=rk3588-buildkit . /opt/rk3588-buildkit

# Keep only what compiling and linking against the sysroot needs: drop
# documentation, translations, the target's own executables and its password
# database (the image is public).
RUN cd /opt/rk3588-buildkit \
    && rm -rf doc man share/doc share/man share/info share/locale share/gtk-doc \
    && cd aarch64-buildroot-linux-gnu/sysroot \
    && rm -rf usr/bin usr/sbin usr/libexec bin sbin \
        usr/share/doc usr/share/man usr/share/info usr/share/locale \
        etc/shadow etc/gshadow

FROM ubuntu:24.04

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl git pkg-config xz-utils \
    && rm -rf /var/lib/apt/lists/*

COPY --from=prune /opt/rk3588-buildkit /opt/rk3588-buildkit

ENV PATH=/opt/rk3588-buildkit/bin:$PATH

LABEL org.opencontainers.image.source="https://github.com/Rust-KVM/rustkvm" \
      org.opencontainers.image.description="RK3588 Buildroot cross toolchain and sysroot for RustKVM"
