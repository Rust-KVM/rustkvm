# Releasing and shipping firmware

How a RustKVM version goes from a draft release to a flashed unit in a box. Building itself is
in [building.md](building.md).

## Versions

| Thing | Format | Source of truth |
| --- | --- | --- |
| Application | `X.Y.Z` (semver) | `version` in the root `Cargo.toml`; tag `vX.Y.Z` |
| Firmware image | `X.Y.Z-YYYYMMDD-<rev>` | `FW_VERSION` in `scripts/build-firmware.sh`, written to `/etc/rustkvm-release` |
| SDK | commit per project | `os/rk3588/manifest.xml` |

The firmware image is what a unit leaves the factory with, and it always embeds a
released application version. Over-the-air application updates from GitHub Releases
are pending (PR #29); until they land, updates mean reflashing or `dev_deploy.sh`.

## Release assets

| Asset | Built by |
| --- | --- |
| `rustkvm_app-aarch64-unknown-linux-gnu` + `.sha256` | `release.yml` (the pair OTA will install) |
| `BUILD_INFO` | `release.yml` |
| `rustkvm-X.Y.Z-source.tar.gz` | `release.yml` via `scripts/gpl-source-bundle.sh` |
| `SHA256SUMS` + `SHA256SUMS.asc` | `release.yml`, signed with the release key |
| `rustkvm-rk3588-<fw>-sd.img.gz`, `-eflasher.img.gz`, `.release` | `scripts/release-firmware.sh` from a local firmware build |
| `rustkvm-rk3588-<fw>.SHA256SUMS` + `.asc` | `scripts/release-firmware.sh` |
| `rustkvm-X.Y.Z-legal-info.tar.gz` | `LEGAL_INFO=1 scripts/release-firmware.sh` (Buildroot license texts and package sources) |

## Release checklist

Nothing is public until step 6; every earlier step only touches a draft.

1. Bump `version` in `Cargo.toml`, update `CHANGELOG.md`, write
   `docs/release-notes/vX.Y.Z.md`, merge to `dev`.
2. Actions → **Release** → *Run workflow* with `X.Y.Z`. It builds the application in the
   published devkit image and creates the draft release `vX.Y.Z` (the tag does not exist
   yet).
3. Build the firmware from the same commit and attach it to the draft:
   ```bash
   scripts/devkit.sh run scripts/build-app.sh
   FW_VERSION=X.Y.Z-$(date -u +%Y%m%d) scripts/devkit.sh firmware
   LEGAL_INFO=1 scripts/release-firmware.sh X.Y.Z
   ```
4. Flash a unit with the eflasher image and run the acceptance test below.
5. Review the draft: assets, notes, `gpg --verify SHA256SUMS.asc SHA256SUMS`.
6. **Publish** the draft on GitHub. That creates the tag `vX.Y.Z` on the drafted commit.

### Release signing key (one-time)

Generate a dedicated signing key on a trusted machine, store the private half only as
the `RELEASE_SIGNING_KEY` Actions secret (and an offline backup), and commit the public
half so users can verify downloads:

```bash
gpg --quick-gen-key "RustKVM Release Signing" ed25519 sign 3y
fpr=$(gpg --list-keys --with-colons "RustKVM Release Signing" | awk -F: '/^fpr/{print $10; exit}')
gpg --armor --export-secret-keys "$fpr" | gh secret set RELEASE_SIGNING_KEY
gpg --armor --export "$fpr" > docs/release-signing-key.asc
```

The workflow needs the key without a passphrase; leave it empty when gpg asks, or
strip it afterwards with `gpg --passwd "$fpr"`. Set `RELEASE_GPG_KEY=$fpr` for
`scripts/release-firmware.sh` so the firmware checksums carry the same signature.
Without the secret the workflow still creates the draft, unsigned, with a warning.

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
| `/userdata` | empty partition; first boot copies the factory binary from `/usr/lib/rustkvm/` |
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
