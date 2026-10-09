# CLAUDE.md

Guidance for AI coding agents working in this repository.

[`AGENTS.md`](AGENTS.md) is the authoritative reference for the module tree, the backend/frontend contract, conventions and gotchas (see [`CONTRIBUTING.md`](CONTRIBUTING.md)). Read it before non-trivial work. When the module tree or build invariants change, update `AGENTS.md`, not this file.

## Build and check

The device binary cross-compiles only (aarch64, self-built `stage2` toolchain pinned in `rust-toolchain.toml`). The `panic_abort` std variant is required because the release profile sets `panic = "abort"`.

```bash
# Frontend first: the app embeds crates/rkvm-web/dist/ at compile time
cd crates/rkvm-web && trunk build --release

# Device binary
cargo +stage2 build -Z build-std=std,panic_abort --target aarch64-unknown-linux-gnu -p rustkvm --bin rustkvm_app --release

# Build and deploy to a device
./dev_deploy.sh -r <device_ip> [-u <user>]

# Pre-push quality gate (matches CI)
cargo +nightly fmt --all
cargo +nightly clippy --workspace --release --all-targets -- -D warnings
cargo +nightly test --workspace
cargo deny check
```

For backend-only work on a host, a stub `crates/rkvm-web/dist/index.html` is enough to satisfy the embed. Tests are integration tests in each crate's `tests/` directory; changing the RPC method set requires updating `app/tests/rpc_methods.txt`.

## Hard rules

- No `#[allow(...)]` or other lint suppressions; fix or remove the offending code.
- No `unwrap()`/`expect()` in production paths; use `anyhow::Result` and `?`.
- Never `.to_vec()` a GStreamer mapped buffer; the video path stays zero-copy.
- Types shared with the frontend live in `rkvm-proto`; do not duplicate them elsewhere.
- Dependency pins noted in the root `Cargo.toml` are deliberate; see AGENTS.md Gotcha 12 before bumping.

The full list of conventions is in AGENTS.md.

## Device safety

The app arms a hardware watchdog. `killall -9` or `fuser -k` on a running `rustkvm_app` reboots the device. Stop it with SIGTERM and wait; `dev_deploy.sh` does this. The only exception is recovering from `Text file busy` on redeploy.
