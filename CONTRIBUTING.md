# Contributing to RustKVM

## Quality gate

Every push / PR to `main` and `dev` runs `.github/workflows/ci.yml` which enforces:

| Job | Command | Purpose |
|---|---|---|
| `rustfmt` | `cargo fmt --all -- --check` | Code formatting |
| `clippy` | `cargo clippy --workspace --release --all-targets -- -D warnings` | Lints (no `#[allow]` allowed; see `AGENTS.md`) |
| `build (host)` | `cargo build --workspace --release --locked` | Host-target compilation |
| `rustdoc` | `cargo doc --workspace --no-deps --document-private-items` (with `-D warnings`) | Doc-comment validity + intra-doc links |
| `cargo audit` | `cargo audit --deny warnings` | RustSec advisory check |
| `cargo deny` | `cargo deny check` | License / bans / sources |

The `CI Success` job at the end is the required status check for branch protection.

## Local development

```bash
# Quick check before pushing
cargo +nightly fmt --all
cargo +nightly clippy --workspace --release --all-targets -- -D warnings
cargo deny check sources licenses bans

# Cross-compile + deploy (requires stage2 toolchain — see AGENTS.md)
./dev_deploy.sh -r <device_ip>
```

## Pull request expectations

1. **No `#[allow(...)]` suppressions.** Fix or remove offending code.
2. **No new `unwrap()` / `expect()`** outside init-time invariants.
3. **No hot-path allocations.** Use `bytes::Bytes` zero-copy patterns; see `pipeline.rs` for the reference implementation.
4. **Periodic loops** must set `MissedTickBehavior::Delay`.
5. **Singletons** (`CloudManager`, `ConfigManager`) accessed via `get_*` functions, not `::new()`.
6. **Log levels:** `error!` for unrecoverable, `warn!` for recoverable, `info!` for lifecycle, `debug!`/`trace!` for per-event detail.
7. **Update `AGENTS.md`** if module tree or build invariants change.

## Release process

1. Bump `version` in workspace `Cargo.toml`.
2. Update `CHANGELOG.md`.
3. Tag: `git tag -a v0.x.y -m "release v0.x.y" && git push origin v0.x.y`.
4. `.github/workflows/release.yml` will produce a cross-compiled `rustkvm_app` artefact and attach it to the GitHub release.

Note: canonical production builds use the project's self-built `stage2` toolchain via `dev_deploy.sh`. The CI release artefact is built with stable nightly + `-Z build-std=std,panic_abort` and is suitable for smoke-testing, not as the canonical production binary.

## Dependency policy

- Workspace deps live in root `[workspace.dependencies]`; sub-crates pull via `<dep>.workspace = true`.
- Major-version bumps require manual review (see `AGENTS.md` Gotcha #11).
- Dependabot opens weekly PRs for patch/minor updates; major bumps are ignored for pinned crates (`socketioxide`, `reqwest`, `webrtc`, `rustls`, `libc`).
- All git deps must be explicitly listed in `deny.toml` under `[sources].allow-git`.
