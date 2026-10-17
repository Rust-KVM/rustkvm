# Quick start: flash and first boot

From a release download to controlling a computer in your browser. Developers building
from source want [getting-started.md](getting-started.md) instead.

## You need

- The RustKVM RK3588 board, its power supply, an HDMI cable and a USB data cable
- A microSD card (8 GB or larger) and a computer to write it
- Ethernet on the same network as the browser you will use

## 1. Write the installer card

Download `rustkvm-rk3588-<version>-eflasher.img.gz` and `SHA256SUMS` from the release.
Write the image to the microSD card with [balenaEtcher](https://etcher.balena.io/)
(it reads `.gz` directly), or on Linux:

```bash
sha256sum -c SHA256SUMS --ignore-missing
gzip -dc rustkvm-rk3588-<version>-eflasher.img.gz | sudo dd of=/dev/sdX bs=4M conv=fsync
```

## 2. Install to the board

1. Power off the board, insert the card, power on.
2. The card installs RustKVM to the internal eMMC. This takes a few minutes; with a
   monitor on the board's HDMI output you can watch the progress.
3. Power off, **remove the card**, power on. The board now boots RustKVM from eMMC.

To run from the card without touching the eMMC, write the `-sd.img.gz` image instead
and leave the card in.

## 3. Connect the target computer

| Cable | From | To |
| --- | --- | --- |
| HDMI | Target's video output | Board's **HDMI IN** |
| USB | Board's **OTG** port | A USB port on the target (keyboard, mouse, virtual media) |
| Ethernet | Board | Your network |
| ATX extension (optional) | Board | Target's front-panel power / reset header |

## 4. First login

1. Open `https://rustkvm.local/`, or the address your router shows for the board.
2. Accept the browser's certificate warning (the certificate is self-signed).
3. The setup wizard asks you to set a device password. Choose no-password mode only
   on a network you fully trust.
4. Click the video to capture keyboard and mouse; the toolbar has Ctrl+Alt+Del,
   fullscreen, quality and audio.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| `rustkvm.local` does not resolve | Use the IP from your router's DHCP list |
| Black video | Target is on and outputs HDMI; try another resolution in the target's OS |
| No keyboard / mouse | The USB cable goes to the board's OTG port and carries data |
| Forgot the password | Reflash, or remove `/userdata/rustkvm/config.toml` over SSH in developer mode |

Firmware version: Settings → About, or `/etc/rustkvm-release` on the board.
