# Getting started on an RK3588 board

This guide takes you from a bare RK3588 board to controlling a target computer in the
browser, with an application you built yourself. It assumes you have built the
application (and, if you want your own image, the firmware) with the Docker devkit as
described in [building.md](building.md):

```bash
scripts/devkit.sh run scripts/build-app.sh   # out/app/
scripts/devkit.sh firmware                   # out/firmware/ (optional)
```

## 1. What you need

| Item | Notes |
| --- | --- |
| RK3588 board | With an **HDMI input** (HDMI-RX) and a USB port that supports **device / OTG mode** |
| RustKVM firmware image | `rustkvm-rk3588-<version>-sd.img.gz` or `-eflasher.img.gz`, from a release or your own `out/firmware/` |
| microSD card | 8 GB or larger |
| HDMI cable | Target's video output → board's HDMI input |
| USB cable | Board's OTG port → a USB port on the target (keyboard, mouse and virtual media) |
| Network | Ethernet or Wi-Fi on the same network as your browser |
| Optional | ATX extension wired to the target's front-panel header, DC power extension |

## 2. Flash the firmware

Write the image to a microSD card exactly as in [quickstart.md](quickstart.md#1-write-the-installer-card):
the `-eflasher` image installs to eMMC, the `-sd` image runs from the card. Boot the
board, find its IP address in your router's DHCP list, open `https://<device_ip>/` and
finish the setup wizard.

The firmware already runs RustKVM, so you can stop here. The next section replaces the
application with one you are working on.

## 3. Deploy RustKVM

SSH is off in the factory image and root password login is disabled, so turn on key
login from the web UI first:

1. Settings → Security → **SSH access**: paste your public key (`~/.ssh/id_ed25519.pub`)
   and save.
2. Settings → System → Developer: tick **Developer mode**. The device starts the SSH
   server; `ssh root@<device_ip>` now works with that key.

Then, from the repository:

```bash
./dev_deploy.sh -r <device_ip>
```

The script builds `rustkvm_app` (with the self-built `stage2` toolchain if you have one,
otherwise in the devkit), stops the running instance gracefully, copies the binary to
`/userdata/rustkvm/bin/` and opens an SSH session on the board. Start the service from
that session:

```bash
/etc/init.d/S99rustkvm restart
tail -f /userdata/rustkvm/log/rustkvm_app.log
```

> [!WARNING]
> `rustkvm_app` arms the hardware watchdog. Stop it with `/etc/init.d/S99rustkvm stop`
> or `killall rustkvm_app` (SIGTERM). `killall -9` or `fuser -k` while it runs makes the
> watchdog reboot the board.

To go back to the application shipped in the image, delete
`/userdata/rustkvm/bin/rustkvm_app` and restart the service; it copies the factory copy
from `/usr/lib/rustkvm/` again.

## 4. Connect the target

1. HDMI cable from the target to the board's HDMI input.
2. USB cable from the board's OTG port to the target.
3. Optionally, wire the ATX extension to the target's power and reset headers.

## 5. First login

1. Open `https://<device_ip>/` or `https://rustkvm.local/` (mDNS).
2. Accept the self-signed certificate. It is generated at every start, so browsers
   warn until you install your own certificate under Settings → TLS.
3. The **setup wizard** asks for a device password, or no-password mode for a trusted
   lab network.
4. Click the video view to capture keyboard and mouse. Press the toolbar buttons for
   Ctrl+Alt+Del, fullscreen, quality and audio.

## 6. Next steps

- **Boot an installer**: Settings → Virtual media → mount an ISO by URL, from device
  storage, or the bundled netboot.xyz image.
- **Power control**: Settings → Power for ATX power / reset and DC power.
- **Automation**: create an API token (`createApiToken`) and use `POST /device/rpc` or
  connect an MCP client to `POST /mcp`. See the [API reference](api.md).
- **Home Assistant**: Settings → MQTT, with discovery enabled.
- **Monitoring**: scrape `/metrics` with Prometheus; check `/device/health`.

## Files on the device

| Path | Contents |
| --- | --- |
| `/userdata/rustkvm/bin/rustkvm_app` | The binary |
| `/userdata/rustkvm/config.toml` | Configuration (a legacy `config.json` is migrated automatically) |
| `/usr/lib/rustkvm/rustkvm_app` | Factory copy in the image, installed when the one above is missing |
| `/userdata/rustkvm/log/rustkvm_app.log` | Log |
| `/userdata/rustkvm/devmode.enable` | Present while developer mode (SSH) is on |
| `/userdata/rustkvm/.enablefailsafe` | Create this file to force failsafe mode on the next start |
| `/tmp/rustkvm/` | TLS certificate and key (not persisted) |

## Troubleshooting

| Symptom | What to check |
| --- | --- |
| Black video | HDMI cable and source resolution; `getVideoState` / `GET /device/health` report signal and errors; `restartVideoPipeline` restarts the pipeline. |
| Keyboard or mouse does nothing | The OTG cable must go to a data-capable port; `getUSBState` should report `configured`. |
| `Text file busy` on redeploy | Run `killall rustkvm_app` and wait before copying. Only if that fails: `killall -9 rustkvm_app; sleep 2`. |
| Browser cannot reach `rustkvm.local` | mDNS may be blocked on your network; use the IP address. |
| `ssh: connect ... refused` | Developer mode is off, or the key was not saved (step 3) |
| Locked out after failed logins | Login is rate-limited with exponential backoff (up to 2 hours); wait or restart the service. |

Still stuck? Open a [bug report](https://github.com/Rust-KVM/rustkvm/issues/new/choose)
with the log and a diagnostics bundle from `GET /diagnostics`.
