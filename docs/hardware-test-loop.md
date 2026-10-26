# Change → build → deploy → verify on hardware

The maintainer's loop for testing a change on a real RK3588 device. Release packages are **not** built in the cloud; only the WSL2 host below produces them. Buildroot/image changes land as code plus build and verify steps, not as prebuilt images.

## Topology

```
Mac (Claude Code Remote Control, ~/Downloads/RustLang/rustkvm)
  └─ ssh foxx@192.168.1.173         Windows jump host (OpenSSH for Windows, default shell cmd.exe)
       └─ wsl.exe → WSL2            repo: /home/foxx/obj/RK3588/jetkvm/rustkvm  (only place packages are built)
Device: ssh root@192.168.1.6        RK3588, dropbear
```

- IPs are LAN addresses and may change after a device reboot. `192.168.1.221` is a different host whose SSH host key changed; do not treat it as the device.
- Mac: key `~/.ssh/id_ed25519`, loaded into the keychain / ssh-agent.
- Windows: `foxx` is an administrator, so authorized keys live in `C:\ProgramData\ssh\administrators_authorized_keys`, not `~/.ssh`. `ssh-copy-id` does not work there; append the public key to that file by hand.
- Device: keys live in `/userdata/dropbear/.ssh/authorized_keys`.
- Never commit passwords or private keys.

## Steps

1. **Build** (in WSL, on the branch under test). If the frontend changed, run `trunk build --release` in `crates/rkvm-web` first, since the app embeds `dist/`.
   ```bash
   ssh foxx@192.168.1.173 'wsl.exe -e bash -lc "cd /home/foxx/obj/RK3588/jetkvm/rustkvm && git status -sb | head -1 && cargo +stage2 build -Z build-std=std,panic_abort --target aarch64-unknown-linux-gnu -p rustkvm --bin rustkvm_app --release"'
   ```
   A cold release build takes several minutes.
2. **Stop the app with SIGTERM** (never `-9`, see below):
   ```bash
   ssh root@192.168.1.6 'pkill -TERM -x rustkvm_app'
   ```
3. **Copy** `target/aarch64-unknown-linux-gnu/release/rustkvm_app` from WSL to `/userdata/rustkvm/bin/rustkvm_app` on the device (scp from WSL, or via the Mac).
4. **Restart**: `ssh root@192.168.1.6 '/etc/init.d/S99rustkvm restart'`
5. **Verify**: tail `/userdata/rustkvm/log/rustkvm_app.log`, and load the web UI at https://192.168.1.6/ (http also serves).

`dev_deploy.sh` is not used in this loop: it uses `fuser -k` (watchdog reboot risk) and ends in an interactive ssh.

## Caveats

- **Watchdog**: `killall -9` / `fuser -k` on a running `rustkvm_app` reboots the device. Only exception: `Text file busy` on redeploy.
- **Strict config**: `/userdata/rustkvm/config.toml` is parsed with `deny_unknown_fields`. A key from a newer or older build crashes startup (deprecated `ai_*` keys are ignored with a warning; others are not). Back the file up (`cp config.toml config.toml.bak.<tag>`) before editing, and remove only the offending key.
- **Harmless log line**: `Failed listening on '22'` comes from the init script's SSH daemon, not the app.
- **Claude Code auto mode on the Mac**: plain `ssh foxx@192.168.1.173 '...'` and `ssh root@192.168.1.6 '...'` are allowed by rules in `~/.claude/settings.json` (rules load only in a new session; do not put options like `-o` before the host, it breaks the prefix match). These get blocked: piping a script into `wsl.exe`, chaining a local command after an ssh command, and running curl from the Mac against the device (curl from the device itself works). cmd.exe quoting is fragile, so a committed deploy script run through one of the allowed prefixes is the preferred next step.

For multi-hour stability and leak runs on top of this loop, see [soak-testing.md](soak-testing.md).
