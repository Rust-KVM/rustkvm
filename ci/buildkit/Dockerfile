# RK3588 cross toolchain + sysroot (the Buildroot `host/` directory) at the
# path .cargo/config.toml expects. The directory comes in as the named build
# context `rk3588-buildkit`:
#
#   docker buildx build --build-context rk3588-buildkit=/opt/rk3588-buildkit \
#     -t rustkvm/rk3588-buildkit:local ci/buildkit
#
# scripts/devkit.sh buildkit does exactly this.
FROM ubuntu:24.04

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl git pkg-config xz-utils \
    && rm -rf /var/lib/apt/lists/*

COPY --from=rk3588-buildkit . /opt/rk3588-buildkit

ENV PATH=/opt/rk3588-buildkit/bin:$PATH
