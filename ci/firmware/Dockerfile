# Host environment for the FriendlyElec RK3588 Buildroot SDK (kernel, u-boot,
# Buildroot rootfs, SD / eMMC image packing). Used by scripts/build-firmware.sh
# through scripts/devkit.sh firmware.
#
# Image packing uses loop devices, so the container runs --privileged.
FROM ubuntu:22.04

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        android-sdk-libsparse-utils bc bison btrfs-progs build-essential ca-certificates \
        ccache clang cmake cpio curl fdisk swig \
        device-tree-compiler dosfstools dwarves e2fsprogs exfatprogs fakeroot file flex \
        gcc-arm-linux-gnueabihf git git-lfs gperf kmod libclang-dev \
        libdbus-1-dev libelf-dev libgmp-dev libmpc-dev libncurses-dev \
        libnl-3-dev libssl-dev llvm-dev lz4 openssl parted patch \
        pkg-config python-is-python3 python3 python3-dev python3-pip python3-pyelftools \
        rsync sudo texinfo \
        u-boot-tools udev unzip uuid-dev wget xz-utils zstd \
    && rm -rf /var/lib/apt/lists/*

# Google's `repo` launcher (v2.69), pinned by content hash.
ARG REPO_SHA256=6ed2f6325705b6ea0af99466d6ebd04adec387a5b6a906cd77bc8c6da65123e5
RUN curl -sSfL https://storage.googleapis.com/git-repo-downloads/repo -o /usr/local/bin/repo \
    && echo "${REPO_SHA256}  /usr/local/bin/repo" | sha256sum -c - \
    && chmod a+rx /usr/local/bin/repo

# Buildroot refuses some packages as root; the build runs as `builder`, which
# may sudo for the SDK's image-packing steps. devkit.sh maps it to the host uid.
ARG UID=1000
ARG GID=1000
RUN groupadd -g "${GID}" builder \
    && useradd -m -s /bin/bash -u "${UID}" -g "${GID}" builder \
    && echo 'builder ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/builder \
    && git config --system --add safe.directory '*' \
    && git config --system user.name "RustKVM firmware build" \
    && git config --system user.email "build@localhost"

USER builder
WORKDIR /home/builder
