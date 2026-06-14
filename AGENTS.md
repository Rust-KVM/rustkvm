# AGENTS.md — RustKVM

## Build: cross-compilation only

This project targets **aarch64-unknown-linux-gnu** (RK3588 ARM64) and uses a **self-built Rust stage2 toolchain** (compiled from `rust/` source). The toolchain is linked via `rustup toolchain link stage2`.

Build pipeline:  `rust/` source → `./x build --stage 2` → `rustup link stage2` → `cargo +stage2 build`

```bash
# Build application (uses self-built stage2 compiler + std from source).
# panic_abort std variant is required because [profile.release] sets panic = "abort".
cargo +stage2 build -Z build-std=std,panic_abort --target aarch64-unknown-linux-gnu -p rustkvm --bin rustkvm_app --release
./dev_deploy.sh -r <target_ip>
```

Strict workspace checks (clippy uses default-toolchain std, no -Z build-std needed):

```bash
cargo +nightly clippy --workspace --release --all-targets -- -D warnings
```

**No clippy suppression is permitted.** `#[allow(dead_code)]`, `#[allow(clippy::…)]`, and equivalent attributes are forbidden — fix or remove the offending code instead. Reference the upstream Go implementation (`/home/foxx/obj/RK3588/jetkvm/kvm`) when behavioural parity is required.

## Workspace layout

Cargo workspace (`resolver = "2"`, `edition = "2024"`, MSRV `1.87`) with one binary crate and two internal infrastructure crates:

```
rustkvm/                     # root workspace
  Cargo.toml                 # members = ["app", "crates/rkvm-core", "crates/rkvm-net"]
  app/                       # binary crate `rustkvm` (lib + bin)
    src/main.rs              # binary entrypoint
    src/lib.rs               # library root
  crates/
    rkvm-core/               # startup wall-clock sync
    rkvm-net/                # mDNS, WoL, Tailscale CLI, local-IP enumeration
  client/static/             # frontend SPA (the ONLY embedded dir, via rust-embed; assets.rs #[folder])
                             # Source: Go kvm/static/ (device build)
  client/dist/  client/dist0/  # non-embedded alternative builds (not used by the binary)
  assets/images/             # disk images → BuiltinImages
  docs/                      # ReleaseNote.md
  tests/                     # WebRTC manual-test HTML pages + bundled tooling
```

**Frontend is read-only.** `client/` mirrors `kvm/static/` from the upstream Go project at `/home/foxx/obj/RK3588/jetkvm/kvm`. Never edit it here — sync from upstream when a refresh is needed.

## Architecture

RustKVM is a KVM-over-IP RK3588 appliance. Compatible with the upstream SPA (device build) — branding strings in the embedded HTML/manifest are RustKVM.

| Concern         | Crate / Module                                                  |
| --------------- | --------------------------------------------------------------- |
| Web server      | `salvo` (routes, middleware, static, Socket.IO)                 |
| Video           | `gstreamer` + Rockchip MPP (H.264/H.265 HW encode)              |
| Streaming       | `webrtc` crate (peer connections, data channels)                |
| Signaling       | `socketioxide` (Socket.IO)                                      |
| TLS             | `rustls` + `rcgen` (self-signed, generated at startup)          |
| mDNS            | `rkvm-net::mdns` (wraps `mdns-sd`)                              |
| Wake-on-LAN     | `rkvm-net::wol`                                                 |
| Tailscale       | `rkvm-net::tailscale` (CLI shell-out)                           |
| Local-IP        | `rkvm-net::local_ip` (`if-addrs`)                               |
| Time sync       | `rkvm-core::time_sync` (best-effort wall-clock sync at startup) |
| MQTT            | `app::mqtt` over `rumqttc` (HA discovery, commands, publish)    |
| JSON-RPC        | `app::api` (120+ handlers + 5 broadcast events incl. `failsafeMode`/`willReboot`/`networkState`) |
| HID-RPC         | `app::hidrpc` (binary protocol over WebRTC data channels, zero-copy `KeyboardReport`) |
| Observability   | `app::observability` (`rustkvm_app_info` + frame/rpc counters + RPC latency histogram + live log-level reload) |
| Failsafe        | `app::failsafe` (env/file/crash-log boot detection)             |
| Rate-limit      | `app::web::ratelimit` (login exponential backoff 5→15m/30m/1h/2h) |
| Power           | `app::power::{atx,dc}` (GPIO + sysfs)                           |
| Auth            | `app::middleware` + `app::web::auth` (noPassword/password, bcrypt, rate-limited login) |
| Cloud           | `app::cloud` (OIDC verifier, WebSocket relay)                   |
| Hardware        | USB gadget, EDID, display (LVGL), watchdog, jiggler, native     |
| Logging         | `tracing`                                                       |

## Module tree

### `app/src/`

```
main.rs                  # CLI → failsafe → tuning → time_sync (+periodic 1h) → config → network (hostname+state monitor)
                         # → watchdog → tls → prometheus → ctrl socket → webrtc → usb → display → video
                         # → virtual media restore → jiggler → mqtt → mdns → web/cloud → serve
lib.rs                   # module roots (see below)
├── cli.rs               # Clap arguments
├── config/              # OnceCell<ConfigManager>, TOML at /userdata/rustkvm/config.toml (auto-migrates legacy JSON)
├── failsafe.rs          # Boot crash-log / env-var / .enablefailsafe trigger + RPC + broadcast event
├── api/                 # JSON-RPC 2.0
│   ├── registry.rs      # RpcRegistry; per-call counter + latency histogram
│   ├── registry_builder.rs
│   ├── handlers/{hid,media,network,system,usb}.rs
│   ├── events.rs        # broadcast events: usbState, keyboardLedState, failsafeMode, willReboot, networkState
│   └── types.rs
├── web/                 # Salvo router (split by concern)
│   ├── routes.rs        # protected / public / developer / static
│   ├── auth.rs          # login (rate-limited), logout, mode switch
│   ├── ratelimit.rs     # login exponential backoff
│   ├── device.rs        # device endpoints (robots.txt, info, cloud state)
│   ├── storage.rs       # virtual-media upload / list
│   ├── socket.rs        # Socket.IO signaling namespace (join, signal, ice-candidate)
│   ├── webrtc_handlers.rs # offer/answer + signaling glue
│   ├── pprof.rs         # `/debug/pprof` (dev only)
│   └── types.rs
├── webrtc.rs            # PeerConnection factory, ICE, data channels (incl. cdcacm route)
├── video.rs             # GStreamer + MPP pipeline; zero-copy `bytes::Bytes::from_owner(MappedBuffer)`
                         # frames into WebRTC RTP — no per-frame memcpy
├── hardware/
│   ├── usb/             # HID (keyboard/mouse), storage (virtual media), gadget, descriptors
│   ├── native/          # Ctrl socket (SEQPACKET), JSON-RPC bridge
│   ├── block_device.rs  # NBD-backed virtual-media block device
│   ├── display.rs       # Backlight + LVGL
│   ├── edid.rs          # EDID parsing + persistence
│   ├── hw.rs            # Hardware probe / capability table, watchdog
│   ├── jiggler.rs       # Mouse jiggler
│   └── tuning.rs        # RK3588 big-core CPU governor / IRQ affinity
├── cloud/               # CloudManager singleton via `get_cloud_manager()`; OIDC, WebSocket relay
├── middleware.rs        # Auth hoop (noPassword/password/cookie), dev mode
├── state.rs             # AppState (sessions, sockets, ICE queue)
├── session.rs           # WebRTC session lifecycle (signaling lives in web/socket.rs + web/webrtc_handlers.rs)
├── data_channel.rs      # WebRTC routes: rpc, disk, terminal, serial, hidrpc(+variants), upload_*, cdcacm
├── tls.rs               # Self-signed certs via rcgen
├── terminal.rs          # PTY/shell
├── pipeline.rs          # Video/audio pipeline (zero-copy)
├── remote_mount.rs      # Remote filesystem mount
├── assets.rs            # rust-embed (ClientAssets, BuiltinImages)
├── dev_mode.rs          # Dev mode marker file + SSH keys
├── network.rs           # NetworkConfig, hostname init, 30s state broadcast
├── hidrpc/              # Binary HID-RPC over WebRTC data channels
│   ├── codec.rs         # Message framing; zero-copy KeyboardReport
│   ├── dispatcher.rs    # In-process routing to USB HID
│   └── mod.rs
├── mqtt/                # rumqttc client + Home Assistant integration
│   ├── manager.rs / lifecycle.rs / commands.rs / publish.rs
│   ├── discovery.rs     # HA discovery payloads
│   ├── rpc.rs / stubs.rs / tls.rs / types.rs
├── observability/       # Prometheus metrics + live log-level reload
│   ├── metrics.rs       # app_info, video/audio frame counters, RPC call counter + latency histogram
│   ├── diagnostics.rs   # `/diagnostics.json`
│   └── mod.rs           # `install_log_reload_handle` + `set_log_filter`
├── version.rs           # build.rs-populated GIT_REVISION/GIT_BRANCH/BUILD_DATE/RUSTC_VERSION via option_env!
└── power/               # ATX/DC power button + LED control
    ├── atx.rs           # ATX GPIO state machine
    ├── dc.rs            # DC sysfs monitoring + GPIO control
    └── mod.rs
```

### `app/build.rs`

Populates `GIT_REVISION`, `GIT_BRANCH`, `BUILD_DATE`, `RUSTC_VERSION` via `cargo:rustc-env`; consumed by `version.rs::VersionInfo` and exported as `rustkvm_app_info`.

### `crates/rkvm-core/src/`

```
lib.rs                   # exposes `error` (Error/Result) + `time_sync`
├── error.rs             # crate Error/Result types
└── time_sync.rs         # NTP-like best-effort sync at startup
```

### `crates/rkvm-net/src/`

```
lib.rs                   # exposes `error` (Error/Result), `local_ip`, `mdns`, `tailscale`, `wol`
├── error.rs             # crate Error/Result types
├── local_ip.rs          # first non-loopback IPv4 via `if-addrs`
├── mdns/                # mDNS-SD service registration wrapper (config.rs, server.rs, mod.rs)
├── tailscale.rs         # `tailscale` CLI shell-out (status, control URL)
└── wol.rs               # Wake-on-LAN magic-packet sender
```

## Frontend integration

The Rust backend communicates with the SPA via:
- **HTTP REST** — Salvo router, split across `web/routes.rs` (protected/public/developer/static)
- **Socket.IO** — real-time signaling (join, signal, ice-candidate, offer)
- **WebRTC** — video streaming (zero-copy `Bytes`) + JSON-RPC over data channels
- **JSON-RPC 2.0** — 120+ method registrations in `api/registry_builder.rs`.

### Frontend build (read-only from rust side)
```bash
# Run from the upstream Go project, NOT from rustkvm/
cd /home/foxx/obj/RK3588/jetkvm/kvm/ui && npm run build:device   # → kvm/static/
rm -rf /home/foxx/obj/RK3588/jetkvm/rustkvm/client/static
cp -r /home/foxx/obj/RK3588/jetkvm/kvm/static /home/foxx/obj/RK3588/jetkvm/rustkvm/client/static
```
**Use `kvm/static/` (device build), NOT `kvm/ui/dist/` (cloud build).**

## Deployment

```bash
./dev_deploy.sh -r <device_ip> -u root   # one-step

# Manual:
cargo +stage2 build -Z build-std=std,panic_abort --target aarch64-unknown-linux-gnu -p rustkvm --bin rustkvm_app --release
scp target/aarch64-unknown-linux-gnu/release/rustkvm_app root@<ip>:/userdata/rustkvm/bin/
```

Default test device: `192.168.1.153` (user `root`).

### Start on device
```bash
ssh root@<ip>
killall rustkvm_app 2>/dev/null
nohup setsid /userdata/rustkvm/bin/rustkvm_app </dev/null >>/userdata/rustkvm/log/rustkvm_app.log 2>&1 &
```
**DO NOT use `fuser -k` or `killall -9`** — watchdog may reboot the device. The only exception is recovering from `Text file busy` on redeploy: `killall -9 rustkvm_app; sleep 2` before scp.

## Conventions

- **Edition 2024**, MSRV `1.87`. `cargo +nightly fmt --all` before commits.
- **Workspace lints** raised to `warn`: `unnecessary_to_owned`, `redundant_clone`, `inefficient_to_string`; `unsafe_op_in_unsafe_fn` for edition-2024 unsafe hygiene. Suppression attributes (`#[allow(...)]`) are not allowed anywhere in the tree.
- **Release profile**: `lto = "fat"`, `codegen-units = 1`, `strip = "symbols"`, `panic = "abort"`, `overflow-checks = false`, `incremental = false`. Final stripped binary ≈ 27 MB on aarch64. Requires `-Z build-std=std,panic_abort`.
- **Error handling**: `anyhow::Result` + `?`. No `unwrap`/`expect` in production paths.
- **Sync primitives**: `parking_lot::Mutex` / `parking_lot::RwLock` for synchronous code. Use `tokio::sync` only when holding a guard across `.await`.
- **Periodic loops**: every `tokio::time::interval` MUST `.set_missed_tick_behavior(MissedTickBehavior::Delay)`. For one-shot delays use `tokio::time::sleep` (NOT `interval(...).tick()` — the first tick is immediate, which silently breaks "fire after N seconds" semantics).
- **Hot-path allocations**: video/audio frames flow via `bytes::Bytes::from_owner(gst::MappedBuffer)`. Never `.to_vec()` a GStreamer mapped buffer. JSON-RPC dispatch uses `serde_json::from_slice(&data)` on the raw `Bytes` — no UTF-8 → String intermediate.
- **Singletons**: `CloudManager`, `ConfigManager` are global. ALWAYS use `cloud::manager::get_cloud_manager()` / `config::get_config_manager()`. Never call `::new()` outside the singleton's init.
- **Tracing**: structured key-value fields, not free-form prose. `debug!`/`trace!` for hot paths, `info!` for lifecycle. Live log-level reload via `observability::set_log_filter()` (`setDefaultLogLevel` RPC or `RUST_LOG` at startup).
- **Global state**: `OnceCell`/`OnceLock`. Panic on uninitialized access is intentional.
- **Comments**: only when the *why* is non-obvious (hidden invariants, hardware quirks, workarounds) and for `unsafe { … }` blocks (`// SAFETY: …`). Section dividers, header banners, and "what this does" comments are forbidden — names should carry the meaning.
- **Tests**: no `#[cfg(test)]` modules. `tests/` holds WebRTC manual-test HTML pages only.

## Gotchas

1. **Cross-compile only** — MUST use `-Z build-std=std,panic_abort --target aarch64-unknown-linux-gnu`. The `panic_abort` std variant is required because the release profile sets `panic = "abort"`.
2. **No C toolchain dependency** — EDID ioctls are pure-Rust via `rustix::ioctl` (`hardware/edid.rs`). Do not reintroduce a build script without a genuinely irreducible C dep.
3. **Config path** — `/userdata/rustkvm/config.toml`. Legacy `config.json` is auto-migrated. Default `local_auth_mode = ""` (empty → frontend setup wizard). Persisted RPC settings: `auto_update_enabled`, `include_pre_release`, `display_rotation`, `display_{max_brightness,dim_after_sec,off_after_sec}`, `network_config.ipv4_mode`. Persisted file: `serialCommandHistory.json`.
4. **Auth middleware** — Salvo hoops auto-call `ctrl.call_next()` on return. Do NOT add it manually.
5. **Error bodies** — Salvo `StatusError` renders empty on 4xx/5xx. Frontend checks status codes, not body.
6. **TLS certs** at `/tmp/rustkvm/` — not persisted across reboots.
7. **Device stability** — `killall -9` triggers watchdog reboot. Use `killall` (SIGTERM) except for the redeploy `Text file busy` case.
8. **Ctrl socket** — `hardware::native::socket::init_ctrl_socket()` MUST be called from `main.rs` after `tls::init()`; otherwise every native bridge call logs `ctrl socket not initialized`. The LVGL companion binary listens on `/var/run/rustkvm_ctrl.sock` (Unix `SEQPACKET`).
9. **Version reporting** — use `version::built_app_version()` and `version::VersionInfo`. Never hard-code version strings in event payloads.
10. **Module tree is authoritative** — check `lib.rs` / `mod.rs` before re-adding modules. Do not reintroduce without a real call-site: `hardware::serial`, `fuse`, `hardware::manager`, `hardware::usb::jiggler`, `util`, `web.rs` (→ `web/`), `jsonrpc.rs` (→ `api/`), `app::mdns` (→ `rkvm-net`), `ota/` (→ `version.rs`), `hardware::native::process`, `rkvm_core::lock`, `rkvm_net::mdns::utils`, `signaling` (signaling now lives in `web/socket.rs` + `web/webrtc_handlers.rs`).
11. **Conservative dep pins** documented in root `Cargo.toml` (post-`cargo update`):
    - `reqwest = 0.12.x` (latest 0.12.28) — `0.13` requires TLS-feature migration; `rustls` feature in 0.13 pulls aws-lc-rs (aarch64 cross-compile hazard).
    - `webrtc = 0.17.1` — latest stable; `0.20.0-alpha.1` is alpha-only.
    - `rustls = 0.23.x` (latest 0.23.40) — `0.24.0-dev` is dev-only.
    - `libc = 0.2.x` — `1.0.0` is alpha.
12. **Internal crates are path deps** (`rkvm-core`, `rkvm-net`). When refactoring shared infra, prefer extending these crates over adding `pub mod` to `app/`.
13. **Zero-copy invariants** — GStreamer→WebRTC path MUST stay zero-copy: `sample.buffer_owned() → into_mapped_buffer_readable() → bytes::Bytes::from_owner(mapped)`. Never `.to_vec()` a GStreamer mapped buffer.
14. **Reboot semantics** — `reboot` RPC emits `willReboot` event, sleeps 250ms, then calls `/sbin/reboot`. Network ipv4-mode changes also emit `willReboot`.
15. **Setup wizard** — `Config::default().local_auth_mode = ""`. `/device/status` returns `is_setup: false` → frontend triggers setup.
