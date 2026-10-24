# Customizing your build

How to ship RustKVM under your own name: device hostname, the name in the web UI, USB
descriptors, the firmware version label and your own build images. Build everything with
the devkit first ([building.md](building.md)); each change below is followed by the same
two commands:

```bash
scripts/devkit.sh run scripts/build-app.sh
FW_VERSION=<your version> scripts/devkit.sh firmware   # only if you ship firmware
```

## Per device, no rebuild

`/userdata/rustkvm/config.toml` on the device overrides the built-in defaults. Edit it
over SSH (developer mode, see [getting-started.md](getting-started.md#3-deploy-rustkvm))
and restart with `/etc/init.d/S99rustkvm restart`.

```toml
[network_config]
hostname = "mykvm"           # system hostname; mDNS keeps the --mdns-hostname name

[usb_config]                 # all five keys are required in this table
vendor_id = "0x1d6b"
product_id = "0x0104"
serial_number = ""
manufacturer = "MyKVM"
product = "MyKVM USB Emulation Device"
```

`rustkvm_app --help` lists every runtime flag; each also reads a `RUSTKVM_*` environment
variable (for example `--mdns-hostname` / `RUSTKVM_MDNS_HOSTNAME`). The firmware starts
the app from `/etc/init.d/S99rustkvm` without flags, so to change a flag for every unit
change the default in the source instead.

## Built-in defaults (rebuild the app)

| What | Where |
| --- | --- |
| mDNS name (`rustkvm.local`) and fallback hostname | `default_value` of `mdns_hostname` in `app/src/cli.rs` |
| Browser tab title | `<title>` in `crates/rkvm-web/index.html` |
| Name in the sidebar, login card and page header | `"rustkvm"` strings in `crates/rkvm-web/src/shell/sidebar.rs` and `crates/rkvm-web/src/app.rs` |
| USB manufacturer, product, VID/PID | `impl Default for UsbConfig` in `app/src/config/types.rs` |
| Virtual media SCSI inquiry string | `inquiry_string` in `app/src/hardware/usb/gadget.rs` |
| MQTT base topic and Home Assistant device | `app/src/mqtt/types.rs`, `app/src/mqtt/discovery.rs` |

Use a USB vendor and product ID you are entitled to before shipping hardware; the default
`0x1d6b:0x0104` is the Linux Foundation multifunction gadget ID.

The MCP `serverInfo.name` (`app/src/mcp.rs`) is asserted by `app/tests/mcp.rs`; change
both together if you rename it.

## Firmware

| What | How |
| --- | --- |
| Version label (`/etc/rustkvm-release`, Settings → About, image file names) | `FW_VERSION=...` on the `firmware` command |
| Root password on the serial console | `RUSTKVM_ROOT_PASSWORD='<crypt hash>'` (`mkpasswd -m sha-512`); unset disables root password login. Keep it out of the repository. |
| Files in the root filesystem | `os/rk3588/rootfs-overlay/` |
| Kernel, Buildroot or board changes | patches under `os/rk3588/patches/`, see [`os/rk3588/README.md`](../os/rk3588/README.md) |

The image files are always named `rustkvm-rk3588-<FW_VERSION>-*.img.gz`; rename them
after `collect` if you need another prefix.

## Your own build images

The devkit pulls `ghcr.io/rust-kvm/*` images anonymously. To build and host your own
(after changing `ci/devkit/Dockerfile` or the toolchain), point the scripts at your
registry and push:

```bash
export RUSTKVM_BUILDKIT_IMAGE=ghcr.io/<you>/rk3588-buildkit:<tag>
export RUSTKVM_DEVKIT_IMAGE=ghcr.io/<you>/rustkvm-devkit:<tag>
scripts/devkit.sh image          # builds the devkit on top of the buildkit image
docker login ghcr.io             # with your own token, never committed
scripts/devkit.sh push
```

In a fork, `buildkit.yml` already publishes under your own owner; `release.yml` names
`ghcr.io/rust-kvm/rustkvm-devkit` directly, so change it there if releases should use
your devkit.
