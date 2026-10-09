# Getting started on an RK3588 board

This guide takes you from a bare RK3588 board to controlling a target computer in the
browser. It assumes you have already built `rustkvm_app` as described in the
[README](../README.md#building-from-source).

## 1. What you need

| Item | Notes |
| --- | --- |
| RK3588 board | With an **HDMI input** (HDMI-RX) and a USB port that supports **device / OTG mode** |
| Buildroot system image | Built from the RK3588 Buildroot SDK; the [`buildkit`](../.github/workflows/buildkit.yml) workflow uploads the images as an artifact |
| HDMI cable | Target's video output → board's HDMI input |
| USB cable | Board's OTG port → a USB port on the target (keyboard, mouse and virtual media) |
| Network | Ethernet or Wi-Fi on the same network as your browser |
| Optional | ATX extension wired to the target's front-panel header, DC power extension |

## 2. Flash the system image

Flash the Buildroot image (the `update.img` or partition images under `rockdev/`)
with the usual Rockchip tools, for example `rkdeveloptool` on Linux or RKDevTool on
Windows, with the board in Maskrom or Loader mode. Your board vendor's flashing guide
applies unchanged.

After the first boot, find the board's IP address (router DHCP list, serial console,
or `ip addr` on the board) and make sure you can `ssh root@<device_ip>`.

## 3. Deploy RustKVM

From the repository, with the frontend already built:

```bash
./dev_deploy.sh -r <device_ip> -u root
```

The script cross-builds `rustkvm_app`, stops a running instance gracefully, copies the
binary to `/userdata/rustkvm/bin/` and opens an SSH session on the board. Start the
service from that session:

```bash
nohup setsid /userdata/rustkvm/bin/rustkvm_app </dev/null >>/userdata/rustkvm/log/rustkvm_app.log 2>&1 &
tail -f /userdata/rustkvm/log/rustkvm_app.log
```

> [!WARNING]
> `rustkvm_app` arms the hardware watchdog. Stop it with `killall rustkvm_app`
> (SIGTERM). `killall -9` or `fuser -k` while it runs makes the watchdog reboot the board.

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
| `/userdata/rustkvm/log/rustkvm_app.log` | Log, when started as above |
| `/userdata/rustkvm/.enablefailsafe` | Create this file to force failsafe mode on the next start |
| `/tmp/rustkvm/` | TLS certificate and key (not persisted) |

## Troubleshooting

| Symptom | What to check |
| --- | --- |
| Black video | HDMI cable and source resolution; `getVideoState` / `GET /device/health` report signal and errors; `restartVideoPipeline` restarts the pipeline. |
| Keyboard or mouse does nothing | The OTG cable must go to a data-capable port; `getUSBState` should report `configured`. |
| `Text file busy` on redeploy | Run `killall rustkvm_app` and wait before copying. Only if that fails: `killall -9 rustkvm_app; sleep 2`. |
| Browser cannot reach `rustkvm.local` | mDNS may be blocked on your network; use the IP address. |
| Locked out after failed logins | Login is rate-limited with exponential backoff (up to 2 hours); wait or restart the service. |

Still stuck? Open a [bug report](https://github.com/Rust-KVM/rustkvm/issues/new/choose)
with the log and a diagnostics bundle from `GET /diagnostics`.
