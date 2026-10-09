# Changelog

All notable changes to RustKVM are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Release tags
are `vX.Y.Z` and must match the workspace version in `Cargo.toml`.

## [Unreleased]

No version has been tagged yet. Everything below is on the `dev` branch and will form
the first release, `0.1.0`.

### Added

- **AI-agent access**: MCP server at `POST /mcp` with screenshot, typing, key combo,
  mouse, ATX / DC power, Wake-on-LAN and generic `rpc_call` tools ([#26]).
- **API tokens**: `Authorization: Bearer` tokens for scripts and agents, stored as a
  SHA-256 hash, managed with `createApiToken` / `getApiTokenState` / `revokeApiToken` ([#25]).
- **HID automation RPCs**: `typeText`, `pressCombo`, `mouseMove`, `mouseClick`, and
  keyboard macro playback ([#24]).
- **Health reporting**: `GET /device/health` and the `getHealth` RPC ([#23]).
- **JSON-RPC over HTTP**: transport-agnostic RPC dispatch and `POST /device/rpc`, with
  integration tests for the RPC method set ([#22]).
- **Screenshots**: `GET /device/screenshot` returns the current frame as JPEG ([#21]).
- **Video self-healing**: a supervisor restarts a dead video pipeline with backoff;
  `restartVideoPipeline` RPC ([#27]).
- **Release automation**: RK3588 buildkit container image and a tag-triggered release
  workflow that publishes `rustkvm_app-aarch64-unknown-linux-gnu` with a checksum ([#30]).
- **CI**: host and aarch64 `clippy -D warnings` jobs gate merges ([#20], [#28]).
- **Web frontend**: `rkvm-web`, a Leptos 0.8 / WebAssembly single-page app, and the
  shared `rkvm-proto` protocol crate.
- Runtime audio and display configuration, host wake, five-button mouse support.
- Hardware watchdog, GStreamer video and audio pipeline with Rockchip MPP encoding, USB
  gadget (HID and mass storage), EDID handling and the CLI.
- Project documentation: README (English and Chinese), API reference, contributing
  guide, security policy, code of conduct, issue forms and PR template ([#32]).

### Changed

- The project is split into a Cargo workspace (`app`, `rkvm-core`, `rkvm-net`,
  `rkvm-proto`) with a modular application layout.
- DHCP lease renewal no longer blocks the RPC handler ([#27]).

### Removed

- Cross-compilation helper scripts and bundled C sources; EDID ioctls are pure Rust.

[Unreleased]: https://github.com/Rust-KVM/rustkvm/commits/dev
[#20]: https://github.com/Rust-KVM/rustkvm/pull/20
[#21]: https://github.com/Rust-KVM/rustkvm/pull/21
[#22]: https://github.com/Rust-KVM/rustkvm/pull/22
[#23]: https://github.com/Rust-KVM/rustkvm/pull/23
[#24]: https://github.com/Rust-KVM/rustkvm/pull/24
[#25]: https://github.com/Rust-KVM/rustkvm/pull/25
[#26]: https://github.com/Rust-KVM/rustkvm/pull/26
[#27]: https://github.com/Rust-KVM/rustkvm/pull/27
[#28]: https://github.com/Rust-KVM/rustkvm/pull/28
[#30]: https://github.com/Rust-KVM/rustkvm/pull/30
[#32]: https://github.com/Rust-KVM/rustkvm/pull/32
