FROM ubuntu:24.04

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl git pkg-config xz-utils \
    && rm -rf /var/lib/apt/lists/*

COPY rk3588-buildkit /opt/rk3588-buildkit

ENV PATH=/opt/rk3588-buildkit/bin:$PATH
