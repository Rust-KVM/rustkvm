# Contributing to RustKVM

Thanks for helping build RustKVM. This guide covers the workflow and the checks your
change has to pass. [`AGENTS.md`](AGENTS.md) is the authoritative reference for the
architecture, module tree, conventions and gotchas; read it before larger changes.

By participating you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to contribute

- **Report a bug** or **request a feature** through the
  [issue templates](https://github.com/Rust-KVM/rustkvm/issues/new/choose).
- **Report a vulnerability** privately, as described in [`SECURITY.md`](SECURITY.md).
- **Send a pull request** against the `dev` branch (the default branch).

## Development setup

| You are changing | You need |
| --- | --- |
| Backend (`app/`, `crates/rkvm-*`) | Nightly Rust and the GStreamer dev packages (`libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev`) on an x86_64 Linux host |
| Frontend (`crates/rkvm-web`) | Stable Rust, the `wasm32-unknown-unknown` target and [Trunk](https://trunkrs.dev) |
| Anything you want to run on a device | The RK3588 cross toolchain and the self-built `stage2` Rust toolchain, see the [README](README.md#3-device-build-aarch64--rk3588) |

The backend embeds `crates/rkvm-web/dist/` at compile time. For backend-only work a stub
is enough:

```bash
mkdir -p crates/rkvm-web/dist && touch crates/rkvm-web/dist/index.html
```

## Before you push

```bash
cargo +nightly fmt --all
cargo +nightly clippy --workspace --release --all-targets -- -D warnings
cargo +nightly test --workspace
cargo deny check
```

For frontend changes, from `crates/rkvm-web`:

```bash
cargo +nightly fmt --all
cargo clippy --locked --all-targets --target wasm32-unknown-unknown -- -D warnings
trunk build --locked
```

Build and deploy to a device:

```bash
./dev_deploy.sh -r <device_ip> [-u root]
```

## Quality gates (CI)

Every push and pull request to `main` and `dev` runs these workflows.

**`.github/workflows/ci.yml`** (required check: `CI Success`)

| Job | Command |
| --- | --- |
| `rustfmt` | `cargo fmt --all -- --check` |
| `cargo deny` | `cargo deny check --all-features` (licenses, bans, sources, advisories) |
| `clippy` | `cargo clippy --workspace --release --all-targets --locked -- -D warnings` |
| `clippy (aarch64)` | the same, with `--target aarch64-unknown-linux-gnu` (type-check only, no link) |
| `cargo test` | `cargo test --workspace --locked` |

**`.github/workflows/web.yml`** runs when `crates/rkvm-web/**` or `crates/rkvm-proto/**`
changes (required check: `Web CI Success`): `rustfmt`, `clippy` for
`wasm32-unknown-unknown`, `trunk build --locked`, and `cargo deny`.

## Pull request expectations

1. **No `#[allow(...)]` suppressions** anywhere in the tree. Fix or remove the offending code.
2. **No new `unwrap()` / `expect()`** in production paths; use `anyhow::Result` and `?`.
3. **No hot-path allocations.** Video and audio frames stay zero-copy
   (`bytes::Bytes::from_owner(MappedBuffer)`); never `.to_vec()` a GStreamer buffer.
4. **Periodic loops** set `MissedTickBehavior::Delay`; one-shot delays use `tokio::time::sleep`.
5. **Singletons** (`CloudManager`, `ConfigManager`) are reached through their `get_*` functions.
6. **Shared protocol types** live in `rkvm-proto`, not duplicated in `app/` or `rkvm-web/`.
7. **Tests** are integration tests in each crate's `tests/` directory, not
   `#[cfg(test)]` modules. Adding or removing an RPC method means updating
   `app/tests/rpc_methods.txt`.
8. **Logging** uses structured `tracing` fields: `error!` unrecoverable, `warn!`
   recoverable, `info!` lifecycle, `debug!` / `trace!` per-event detail.
9. **Comments** explain only a non-obvious *why*; every `unsafe` block has a `// SAFETY:` note.
10. **Update `AGENTS.md`** when the module tree or a build invariant changes.

Keep pull requests focused on one change, and fill in the pull request template.

## Dependency policy

- Workspace dependencies live in the root `[workspace.dependencies]`; member crates use
  `<dep>.workspace = true`.
- Some crates are deliberately held back (`reqwest` 0.12, `webrtc`, `rustls` 0.23,
  `libc` 0.2, `socketioxide`); the reasons are documented in the root `Cargo.toml` and
  in `AGENTS.md`. Major bumps of these need a manual review.
- Dependabot opens weekly grouped PRs for patch and minor updates of both lockfiles and
  of GitHub Actions.
- Every git dependency must be listed in `deny.toml` under `[sources].allow-git`.

## Release process

1. Bump `version` in the workspace `Cargo.toml`.
2. Tag the commit: `git tag -a vX.Y.Z -m "release vX.Y.Z" && git push origin vX.Y.Z`.
   The tag must equal the workspace version.
3. `.github/workflows/release.yml` builds the frontend, cross-builds `rustkvm_app` in the
   `rk3588-buildkit` image, and publishes `rustkvm_app-aarch64-unknown-linux-gnu` with a
   `.sha256` checksum to the GitHub release.

## License

By contributing you agree that your contributions are licensed under
[GPL-2.0-only](LICENSE), the license of this project.
