# RustKVM API reference

RustKVM exposes one set of device operations, a JSON-RPC 2.0 method registry, over
several transports: WebRTC data channels (used by the web UI), plain HTTP, MQTT and the
Model Context Protocol. This page covers how to reach each one.

The machine-readable HTTP schema is served by the device at `/api-doc/openapi.json`,
with an interactive Scalar UI at `/scalar-ui`.

## Ports and TLS

The web server listens on **443 (HTTPS)** and **80 (HTTP)** on all interfaces, or on
`localhost` only when *local loopback only* is enabled (`setLocalLoopbackOnly`). HTTPS
uses a self-signed certificate generated at startup unless a custom one is configured;
pass `-k` to `curl` or trust the certificate explicitly.

## Authentication

| Mode | How a request is authorised |
| --- | --- |
| `noPassword` | Every request is allowed. |
| `password` | The `authToken` cookie from `POST /auth/login-local`, **or** an `Authorization: Bearer <token>` header carrying an API token. |

On a fresh device `local_auth_mode` is empty and `/device/status` returns
`{"isSetup": false}`; the web UI then shows the setup wizard (`POST /device/setup`).
Failed logins are rate-limited with exponential backoff.

### API tokens

API tokens are meant for scripts and AI agents. A token looks like `rkvm_<64 hex>`, is
shown once when created, and only its SHA-256 hash is stored in the config.

| RPC method | Effect |
| --- | --- |
| `createApiToken` | Create (or replace) the token; returns `{ "token": "rkvm_…" }` once |
| `getApiTokenState` | `{ "enabled": bool, "createdAt": string \| null }` |
| `revokeApiToken` | Delete the token |

Create one from a logged-in browser session (or in `noPassword` mode):

```bash
curl -k https://rustkvm.local/device/rpc -b "authToken=<cookie>" \
  -d '{"jsonrpc":"2.0","id":1,"method":"createApiToken"}'
```

## HTTP endpoints

### Public

| Method | Path | Description |
| --- | --- | --- |
| `GET` | `/device/status` | Whether first-run setup is complete |
| `POST` | `/device/setup` | Complete first-run setup (password or no-password mode) |
| `POST` | `/auth/login-local` | Log in with the device password; sets the `authToken` cookie |
| `GET` | `/metrics` | Prometheus metrics |
| `GET` | `/robots.txt` | Disallows crawling |

### Protected (session cookie or API token)

| Method | Path | Description |
| --- | --- | --- |
| `POST` | `/device/rpc` | JSON-RPC 2.0 request (single call) |
| `POST` | `/mcp` | Model Context Protocol endpoint |
| `GET` | `/device` | Device information |
| `GET` | `/device/health` | Aggregated health report |
| `GET` | `/device/screenshot` | Current screen as `image/jpeg` (503 when no video signal) |
| `POST` | `/device/send-wol/{mac_addr}` | Send a Wake-on-LAN magic packet |
| `POST` | `/webrtc/session` | WebRTC offer / answer exchange |
| `GET` | `/webrtc/signaling/client` | WebRTC signaling |
| `POST` | `/storage/upload` | Upload a virtual-media image |
| `GET` | `/diagnostics` | Download a diagnostics bundle |
| `POST` | `/auth/logout` | End the session |
| `POST` / `PUT` | `/auth/password-local` | Set or change the device password |
| `DELETE` | `/auth/local-password` | Remove the password (switch to no-password mode) |
| `POST` | `/cloud/register` | Register the device with a cloud relay |
| `GET` | `/cloud/state` | Cloud relay state |

### Developer mode only

`/developer/pprof/*` serves Go-pprof-compatible profiling endpoints (`profile`, `heap`,
`allocs`, `trace`, …) once developer mode is enabled.

## JSON-RPC 2.0

The registry holds 130+ methods. The authoritative list is
[`app/tests/rpc_methods.txt`](../app/tests/rpc_methods.txt); the `list_rpc_methods` MCP
tool returns it from a running device.

```bash
curl -k https://rustkvm.local/device/rpc \
  -H "Authorization: Bearer $RUSTKVM_TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"typeText","params":{"text":"hello\n"}}'
```

Common methods by area:

| Area | Methods |
| --- | --- |
| Health and system | `ping`, `getHealth`, `getDeviceID`, `getLocalVersion`, `reboot`, `getFailsafeMode`, `setDefaultLogLevel` |
| Keyboard | `typeText`, `pressCombo`, `keyboardReport`, `keypressReport`, `executeKeyboardMacro`, `getKeyboardLedState`, `setKeyboardLayout` |
| Mouse | `mouseMove`, `mouseClick`, `absMouseReport`, `relMouseReport`, `wheelReport`, `setJigglerState` |
| Video | `getVideoState`, `setStreamQualityFactor`, `setVideoCodecPreference`, `restartVideoPipeline`, `getEDID`, `setEDID` |
| Virtual media | `mountWithHTTP`, `mountWithStorage`, `mountBuiltInImage`, `unmountImage`, `listStorageFiles`, `getVirtualMediaState` |
| Power | `setATXPowerAction`, `getATXState`, `setDCPowerState`, `getDCPowerState`, `sendWOLMagicPacket` |
| USB | `getUSBState`, `setUsbDevices`, `setMassStorageMode`, `setUsbEmulationState` |
| Network | `getNetworkState`, `setNetworkSettings`, `renewDHCPLease`, `getTailscaleStatus`, `setTLSState` |
| MQTT | `getMqttSettings`, `setMqttSettings`, `getMqttStatus`, `testMqttConnection` |

### Events

Over the WebRTC `rpc` data channel the device also pushes JSON-RPC notifications:
`usbState`, `keyboardLedState`, `failsafeMode`, `willReboot` and `networkState`.

### Health report

`GET /device/health` and the `getHealth` method return:

```json
{
  "status": "ok",
  "issues": [],
  "timestamp": "2026-10-09T06:00:00Z",
  "systemUptimeSecs": 12345.6,
  "version": { "version": "0.1.0", "revision": "…", "branch": "dev", "buildDate": "…", "platform": "…" },
  "video": { "ready": true, "width": 1920, "height": 1080, "fps": 60.0, "framesTotal": 1234567, "restarts": 0 },
  "audioFramesTotal": 98765,
  "usbState": "configured"
}
```

`status` is `ok` or `degraded`; `issues` explains a degraded state.

## MCP (Model Context Protocol)

`POST /mcp` implements MCP over Streamable HTTP with JSON responses
(`initialize`, `tools/list`, `tools/call`). It uses the same authentication as the rest
of the API, so give the client an API token.

```bash
claude mcp add --transport http rustkvm https://rustkvm.local/mcp \
  --header "Authorization: Bearer $RUSTKVM_TOKEN"
```

| Tool | Description |
| --- | --- |
| `screenshot` | Capture the current HDMI screen (returned as a JPEG image block) |
| `get_health` | Video signal, USB gadget, network, failsafe and version state |
| `type_text` | Type text on the target (US layout, up to 10 000 characters) |
| `press_combo` | Press a key combination such as `ctrl+alt+delete`, `win+r`, `enter`, `f12` |
| `mouse_move` | Move the absolute pointer |
| `mouse_click` | Move the pointer and click `left`, `right` or `middle` |
| `atx_power` | Press the ATX button: `power-short`, `power-long` or `reset` |
| `get_atx_state` | Read the ATX power and HDD LED state |
| `dc_power` | Switch the DC power extension on or off |
| `get_dc_power_state` | Read the DC power extension state |
| `wake_on_lan` | Send a Wake-on-LAN magic packet |
| `list_rpc_methods` | List every device JSON-RPC method |
| `rpc_call` | Call any device JSON-RPC method by name |

Pointer coordinates are video pixels by default, or absolute HID units (`0..32767`) when
`normalized` is `true`.

## WebRTC data channels

The web UI opens a peer connection via `POST /webrtc/session` (Socket.IO on `/` carries
signaling) and uses these data channels:

| Label | Purpose |
| --- | --- |
| `rpc` | JSON-RPC requests, responses and events |
| `disk` | Virtual media served from the browser (`mountWithWebRTC`) |
| `hidrpc` | Binary HID-RPC, reliable |
| `hidrpc-unreliable-ordered`, `hidrpc-unreliable-nonordered` | Binary HID-RPC for latency-sensitive pointer traffic |
| `terminal` | Device shell |
| `serial` | Serial console |
| `cdcacm` | USB CDC-ACM serial |
| `upload_*` | Virtual-media upload streams |

### HID-RPC framing

HID-RPC is defined in [`crates/rkvm-proto/src/hidrpc.rs`](../crates/rkvm-proto/src/hidrpc.rs)
(protocol version `0x01`). Each message starts with a one-byte type:

| Type | Byte | Direction |
| --- | --- | --- |
| `Handshake` | `0x01` | both |
| `KeyboardReport` | `0x02` | browser → device |
| `PointerReport` | `0x03` | browser → device |
| `WheelReport` | `0x04` | browser → device |
| `KeypressReport` | `0x05` | browser → device |
| `MouseReport` | `0x06` | browser → device |
| `KeyboardMacroReport` | `0x07` | browser → device |
| `CancelKeyboardMacroReport` | `0x08` | browser → device |
| `KeypressKeepAliveReport` | `0x09` | browser → device |
| `KeyboardLedState` | `0x32` | device → browser |
| `KeydownState` | `0x33` | device → browser |
| `KeyboardMacroState` | `0x34` | device → browser |

## MQTT and Home Assistant

Configure the broker with `setMqttSettings` (or in Settings → MQTT). With discovery
enabled the device announces itself under `homeassistant/…/rustkvm_<device id>/…`:

- buttons: ATX power short, ATX power long, ATX reset
- binary sensors: power LED, HDD LED, DC power
- switch: DC power
- sensors: DC voltage, current, power

## Prometheus metrics

`GET /metrics` (public) exports:

| Metric | Type |
| --- | --- |
| `rustkvm_app_info` | gauge with build labels |
| `rustkvm_video_frames_total` | counter |
| `rustkvm_audio_frames_total` | counter |
| `rustkvm_video_pipeline_restarts_total` | counter |
| `rustkvm_rpc_calls_total{method}` | counter |
| `rustkvm_rpc_latency_seconds` | histogram |
