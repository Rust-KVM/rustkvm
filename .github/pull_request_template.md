## Summary

<!-- What does this change, and why? Link the issue it closes, e.g. "Closes #12". -->

Before:

After:

## How

<!-- The approach in a few sentences. Call out anything a reviewer should look at closely. -->

## Testing

<!-- Commands you ran and, for device-facing changes, what you checked on hardware. -->

- [ ] `cargo +nightly fmt --all`
- [ ] `cargo +nightly clippy --workspace --release --all-targets -- -D warnings`
- [ ] `cargo +nightly test --workspace`
- [ ] Frontend touched: `trunk build --locked` and wasm32 clippy in `crates/rkvm-web`
- [ ] Tested on an RK3588 device (describe below), or not applicable

## Checklist

- [ ] No `#[allow(...)]` suppressions and no new `unwrap()` / `expect()` in production paths
- [ ] Hot paths stay zero-copy; periodic loops use `MissedTickBehavior::Delay`
- [ ] Shared protocol types live in `rkvm-proto`
- [ ] RPC method set changed → `app/tests/rpc_methods.txt` updated
- [ ] Module tree or build invariants changed → `AGENTS.md` updated
