# RK3588 firmware layer

Everything that turns the public FriendlyElec RK3588 Buildroot SDK into the RustKVM
system image. The SDK itself is never forked or vendored; `scripts/build-firmware.sh`
checks it out at the commits pinned here and applies the rest.

| Path | Contents |
| --- | --- |
| `manifest.xml` | `repo` manifest: every SDK project at an exact commit on `github.com/friendlyarm` |
| `patches/<project>/NNNN-*.patch` | Changes to SDK projects, applied in order with `git apply` |
| `rootfs-overlay/` | Files copied into the root filesystem (init script, network config) |

The build also adds two things that are not in the tree: the `rustkvm_app` binary
(installed to `/userdata/rustkvm/bin/`) and `/etc/rustkvm-release` with the firmware
version.

## What the patches do

| Project | Change |
| --- | --- |
| `buildroot` | dhcpcd privsep seccomp allows `dup3` (SIGSYS at boot); chrony drift file ownership |
| `buildroot` | RustKVM defconfig (GStreamer Opus, RGA, uvc-gadget, dropbear client, update engine); uvc-gadget builds with `-Dwerror=false` and accepts CONTINUOUS frame intervals |
| `kernel` | `nanopi6_linux_defconfig`: NBD built in (virtual media), USB UAC2 gadget |
| `scripts/sd-fuse` | builds the `v4l2loopback` module (operator-camera passthrough) |
| `external` | rockit links `librkAlgoDis.so` |
| `device/friendlyelec` | installs `rustkvm-files` (this overlay) and disables the stock dropbear service |

## Changing the SDK

Work in the SDK checkout the script created (`$RUSTKVM_SDK_DIR`, default
`~/rustkvm-sdk/rk3588`), then bring the change back here:

```bash
# one project, uncommitted change → new patch
git -C ~/rustkvm-sdk/rk3588/kernel diff > os/rk3588/patches/kernel/0002-short-description.patch

# committed changes on top of the pinned commit
git -C ~/rustkvm-sdk/rk3588/buildroot format-patch --no-signature \
    -o os/rk3588/patches/buildroot --start-number 3 HEAD~1
```

Rerun `scripts/build-firmware.sh patch build collect`; the patch step resets each
patched project to its pinned commit first, so a patch that no longer applies fails
loudly instead of stacking.

To move to newer SDK commits, edit the `revision` of the project in `manifest.xml`
(the `upstream` attribute names the branch it follows), run the `sync` and `patch`
steps, and refresh any patch that stops applying.
