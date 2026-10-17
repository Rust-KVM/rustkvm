# Building RustKVM

Two build products, both reproducible on any x86_64 Linux host (WSL2 included) with
only Docker installed:

| Product | Command | Output |
| --- | --- | --- |
| Application (`rustkvm_app` with the embedded web UI) | `scripts/devkit.sh run scripts/build-app.sh` | `out/app/` |
| Firmware image (kernel, u-boot, rootfs, application) | `scripts/devkit.sh firmware` | `out/firmware/` |

The scripts also run directly on a host that has the tools installed; the containers
just pin those tools.

## Pinned versions

| What | Version | Where it is pinned |
| --- | --- | --- |
| Rust (device binary and frontend) | `nightly-2026-06-14` (rustc 1.98.0-nightly) with `aarch64` and `wasm32` targets | `ci/devkit/Dockerfile`, `release.yml` |
| Trunk | `0.21.14` | `ci/devkit/Dockerfile`, `release.yml`, `web.yml` |
| Cross gcc + sysroot | Buildroot host dir of the firmware below | `rk3588-buildkit` image |
| FriendlyElec Buildroot SDK | exact commit per project | `os/rk3588/manifest.xml` |
| SDK changes | patch files | `os/rk3588/patches/` |
| `repo` launcher | v2.69, sha256-checked | `ci/firmware/Dockerfile` |

The repository's `rust-toolchain.toml` names `stage2`, a self-built toolchain that only
the reference workbench has. The devkit sets `RUSTUP_TOOLCHAIN`, which takes precedence,
so nobody else needs to build rustc.

## 1. Application

```bash
scripts/devkit.sh run scripts/build-app.sh
```

The first run pulls the published devkit image,
`ghcr.io/rust-kvm/rustkvm-devkit:nightly-2026-06-14` (about 3 GB unpacked). `out/app/` then
holds `rustkvm_app-aarch64-unknown-linux-gnu`, its `.sha256` (the same pair the OTA
updater installs) and `BUILD_INFO`.

The devkit image sits on `ghcr.io/rust-kvm/rk3588-buildkit:20250527`, the cross gcc and
sysroot extracted from the firmware build with documentation and target executables
removed. To build both images yourself instead of pulling them:

```bash
scripts/devkit.sh buildkit [HOST_DIR]   # default /opt/rk3588-buildkit, or
                                        # ~/rustkvm-sdk/rk3588/buildroot/output/rockchip_rk3588/host
scripts/devkit.sh image
```

Maintainers publish new images with `scripts/devkit.sh push` after bumping the tags in
`scripts/devkit.sh`, `ci/devkit/Dockerfile` and `.devcontainer/devcontainer.json`.

Day-to-day commands inside the devkit:

```bash
scripts/devkit.sh shell
cargo clippy --workspace --release --all-targets -- -D warnings
cargo test --workspace
cd crates/rkvm-web && trunk serve        # UI dev server on :9000
```

VS Code / any devcontainer client: open the repository and choose *Reopen in
Container*; `.devcontainer/devcontainer.json` uses the same image.

Deploy to a board on the network as before: `./dev_deploy.sh -r <device_ip>`.

## 2. Firmware image

Costs of the first build (the disk figure is the reference workbench's SDK tree; network and time are estimates):

| | |
| --- | --- |
| Disk | about 60 GB for the SDK checkout and build tree, plus about 10 GB of download and ccache under `~/.cache/rustkvm` |
| Network | about 15 GB on the first sync (shallow clones plus Buildroot package sources) |
| Time | several hours on 16 threads; later builds reuse the tree and ccache |

```bash
scripts/devkit.sh run scripts/build-app.sh     # the image embeds out/app/
scripts/devkit.sh firmware                     # sync, patch, build, collect
```

Steps can be run separately: `scripts/devkit.sh firmware patch build collect` skips the
sync. The SDK checkout lives in `RUSTKVM_SDK_DIR` (default `~/rustkvm-sdk/rk3588`) and
belongs to the script: the patch step resets every patched project to its pinned commit.

`out/firmware/` then holds:

| File | Use |
| --- | --- |
| `rustkvm-rk3588-<version>-sd.img.gz` | Boots from a microSD card |
| `rustkvm-rk3588-<version>-eflasher.img.gz` | microSD card that writes the system to eMMC |
| `rustkvm-rk3588-<version>.release` | The `/etc/rustkvm-release` baked into the image |
| `rustkvm-rk3588-<version>.SHA256SUMS` | Checksums of the above |

`<version>` is `<app version>-<UTC date>-<git short rev>`, e.g. `0.1.0-20261017-1a2b3c4d`;
set `FW_VERSION` to override it for a release.

The firmware container runs `--privileged` because the SDK packs images through loop
devices. What the SDK changes are and how to edit them: [`os/rk3588/README.md`](../os/rk3588/README.md).

## Networks behind a proxy

`scripts/devkit.sh` forwards `http_proxy`, `https_proxy` and `no_proxy` into image builds
and containers and uses host networking, so a proxy on `127.0.0.1` works. Base images
come from Docker Hub; if the Docker daemon itself cannot reach it, configure a registry
mirror or proxy for the daemon, or import the Ubuntu base rootfs:

```bash
curl -LO https://cdimage.ubuntu.com/ubuntu-base/releases/24.04/release/ubuntu-base-24.04.5-base-amd64.tar.gz
docker import -c 'CMD ["/bin/bash"]' ubuntu-base-24.04.5-base-amd64.tar.gz ubuntu:24.04
# and the same for 22.04 (firmware builder)
```

## Without containers

The scripts are plain bash. `build-app.sh` needs `trunk`, a nightly with `rust-src`
and the `aarch64-unknown-linux-gnu` target, and `/opt/rk3588-buildkit`.
`build-firmware.sh` needs `repo` and the packages listed in `ci/firmware/Dockerfile`
(Ubuntu 22.04 is what FriendlyElec supports).
