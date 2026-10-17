# Releasing and shipping firmware

How a RustKVM version goes from a tag to a flashed unit in a box. Building itself is
in [building.md](building.md).

## Versions

| Thing | Format | Source of truth |
| --- | --- | --- |
| Application | `X.Y.Z` (semver) | `version` in the root `Cargo.toml`; tag `vX.Y.Z` |
| Firmware image | `X.Y.Z-YYYYMMDD-<rev>` | `FW_VERSION` in `scripts/build-firmware.sh`, written to `/etc/rustkvm-release` |
| SDK | commit per project | `os/rk3588/manifest.xml` |

The application updates itself over the air from GitHub Releases (`app/src/update.rs`);
the firmware image is what a unit leaves the factory with. A firmware release therefore
always embeds a released application version.

## Release checklist

1. Bump `version` in `Cargo.toml`, update `CHANGELOG.md`, merge to `main`.
2. Tag `vX.Y.Z`. `release.yml` builds the application and publishes the OTA assets.
3. Build the firmware from that tag with the released binary:
   ```bash
   git checkout vX.Y.Z
   scripts/devkit.sh run scripts/build-app.sh
   FW_VERSION=X.Y.Z-$(date -u +%Y%m%d) scripts/devkit.sh firmware
   ```
   Check that `out/app/*.sha256` matches the one on the GitHub release.
4. Flash a unit with the eflasher image and run the acceptance test below.
5. Archive `out/firmware/` (images, `.release`, `SHA256SUMS`) with the release; it is
   what the production line flashes.

## Flashing

| Method | Image | When |
| --- | --- | --- |
| microSD boot | `*-sd.img.gz` | Development, field recovery |
| eMMC via eflasher card | `*-eflasher.img.gz` | Production: write to a card, boot the unit from it, it installs to eMMC |
| USB (Maskrom) | partition images under the SDK's `out/` | Bricked units; RKDevTool or `rkdeveloptool` |

Write a card with `balenaEtcher`, or `gzip -dc <img.gz> | sudo dd of=/dev/sdX bs=4M conv=fsync`.

## Factory state

What a freshly flashed unit looks like, and what "factory reset" must restore:

| Item | Factory state |
| --- | --- |
| `/userdata/rustkvm/config.toml` | absent → setup wizard on first login (device password or no-password mode) |
| TLS certificate | self-signed, generated at start |
| SSH (dropbear) | off; enabled only by `/userdata/rustkvm/devmode.enable` |
| Root password (serial console) | login disabled unless the build sets `RUSTKVM_ROOT_PASSWORD` (a crypt hash, kept out of the repository) |
| Hostname / mDNS | `rustkvm.local` |
| `/etc/rustkvm-release` | firmware version, app version, git revision |

## Acceptance test per release

- Boots to the web UI from eMMC; `/etc/rustkvm-release` shows the expected version.
- Setup wizard appears; password login works afterwards.
- HDMI video at the target's native mode, keyboard and mouse, audio.
- Virtual media mounts an ISO; ATX power/reset if the extension is fitted.
- OTA check reports the current version as up to date.
- `GET /device/health` is `ok`; rebooting twice keeps the configuration.
