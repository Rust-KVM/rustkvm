# Soak and leak testing on hardware

How the October 2026 stability pass was run: a functional pass over every JSON-RPC method and MCP tool, then multi-hour soaks with all features running at once, with leak verdicts drawn from trend data. Use it to repeat the pass after a change that touches the HID, video, WebRTC, USB gadget, storage or OCR paths.

The test harness itself is throwaway and is **not** kept in this repository. This page records the procedure, the criteria and the pitfalls, so a new harness can be written from it. Only fixes for problems a run actually finds go to `dev`.

The pass used build `3fa603c` for the 7 h baseline and `7d394aa` (merged as `47dde28`, PR #85) for the 3 h and 5.9 h re-runs.

## Requirements

- A Linux workstation on the device's LAN. The pass used the WSL2 host from [hardware-test-loop.md](hardware-test-loop.md).
- Passwordless `ssh root@<device>`, usable in batch mode.
- HTTP to the device that bypasses any local proxy (`curl --noproxy '*'`).
- A controlled host that may be operated freely. The soak types, clicks, rebinds USB and mounts media on it (see [Side effects on the controlled host](#side-effects-on-the-controlled-host)).
- Python standard library and Node (built-in `fetch`/WebSocket) for the harness, plus a headless Chromium for the viewer.

## 1. Build and deploy

1. Build with `scripts/devkit.sh run scripts/build-app.sh` (output in `out/app/`), or with the manual toolchain from [CLAUDE.md](../CLAUDE.md).
2. Stop the app with `kill -TERM $(pidof rustkvm_app)` and wait up to 40 s for it to exit. Never use `-9`: the watchdog reboots the device. The slim image has no `pkill`.
3. Back up the old binary and the config:
   - `cp rustkvm_app rustkvm_app.pre-soak`
   - `cp config.toml config.toml.bak.pre-soak`
4. Copy the new binary with `scp -O`, since the device has no sftp-server.
5. Run `/etc/init.d/S99rustkvm restart >/dev/null 2>&1 </dev/null`. Without the redirects, ssh hangs.
6. Smoke test within 30 s: toggle CapsLock twice with `keyboardReport` (usage `0x39`). `getKeyboardLedState` must follow, and the app must stay up.

## 2. Functional matrix

- Call every method in `app/tests/rpc_methods.txt`. A method with no test entry fails the run, and so does a test entry for a method that is not in the list.
- Call getters directly. Test setters by writing back the current value: read, set the same value, read again, and require the two reads to match.
- Skip, with a recorded reason, anything that would:
  - change networking
  - flash firmware or factory-reset
  - drive ATX/DC power
  - change SSH keys, API tokens or cloud settings
  - reboot the device
- For MCP, call `initialize`, `ping` and `tools/list`, then call every tool. A tool listed by `tools/list` that has no test case fails the run.
- Classify each result as one of three:
  - `pass`
  - `fail`: a product bug
  - `harness`: the test sent the wrong parameter shape. Recognise these by `invalid type`, `missing field`, `Parameter conversion failed` or `Invalid log level` in the error, and fix the test, not the product.
- Extra check for upload fds: call `startStorageFileUpload`, then `deleteStorageFile("<name>.incomplete")`, then count `.incomplete` entries under `/proc/<pid>/fd`. It must be 0.
- Parameter-shape pitfalls:
  - `emitTestLog` takes an upper-case level.
  - `setUSBState` takes a bare string, sent as `{"params": v}`.
  - `setWakeOnLanDevices` takes `{"params":{"devices":[…]}}`.
  - The registry unwraps single-key objects.

The result on `7d394aa` was 138 pass, 2 fail, 24 skip. The two fails are `getATXState` and MCP `get_atx_state` with no ATX extension attached.

## 3. Load workers

All workers run concurrently, one thread each.

| Worker | Period | What it does |
| --- | --- | --- |
| hid | ~40/s | `absMouseReport` random position (40 %), `relMouseReport` ±20 (30 %), `wheelReport` ±1 (5 %), the rest `keyboardReport` random a–z press/release; plus `keypressReport` Backspace |
| hid_high | 2 s | `typeText` 6 random letters (`delayMs` 5), 6× `pressCombo` backspace, `mouseMove` random; 20 %: `mouseClick` at the centre then `pressCombo` escape |
| mcp | 0.5 s | `initialize`, `tools/list`, `get_health`, `get_logs`, `screenshot`, `mouse_move`, `type_text`, `press_combo`, `get_atx_state`, `get_dc_power_state`, `list_rpc_methods`, `rpc_call(getVideoState)` |
| ocr | 5 s | Rotates `readScreenText`, `findScreenText`, `waitScreenStable`, `waitForText` (absent text), MCP `read_screen_text` / `find_text`, `readScreenText` with `detail=full` |
| screenshot | 2 s | `GET /device/screenshot` |
| settings | 10 s | Writes back the current jiggler config, keyboard layout, quality, macros, WOL list, log level, video sleep and serial history; toggles `setJigglerState` |
| burst | 10 min | 8 concurrent clients hammer read-only methods for 60 s (`ping`, `getHealth`, `getVideoState`, `getUSBState`, `getKeyboardLedState`, `getNetworkState`, `getVirtualMediaState`) |
| macro | 2 min | `executeKeyboardMacro` (KeyA/Backspace ×5); every other run is followed by `cancelKeyboardMacro` |
| media | 10 min | `mountBuiltInImage(netboot.xyz)`, read state after 20 s, `unmountImage` |
| usb | 30 min, from 5 min | `setUsbEmulationState(false)`, `true` 5 s later, then a CapsLock LED check 15 s after that |
| video | 30 min, from 15 min | `restartVideoPipeline` |
| viewer | continuous | See section 5 |

The encoder's valve stays closed while nobody is watching, so fps reads 0 and the video path is idle. A soak without a viewer misses video-path leaks.

## 4. Sampling

Sample every 120 s over ssh (every 30 s for an idle baseline):

- `/proc/<pid>/status`: `VmRSS`, `VmHWM`, `VmSize`, `RssAnon`, `RssFile`, `RssShmem`, `Threads`
- `Pss` from `/proc/<pid>/smaps_rollup`; the entry count of `/proc/<pid>/fd`; the line count of `/proc/<pid>/maps`
- dmabuf: lines of `/proc/<pid>/maps` matching `dmabuf|dma_buf|/dev/dri|rknpu|dma-heap`. For a breakdown, use `size` and `exp_name` in `/proc/<pid>/fdinfo/<fd>` and `/sys/kernel/debug/dma_buf/bufinfo`.
- CPU from the `utime + stime` delta in `/proc/<pid>/stat`; temperature from `thermal_zone0/temp`; `/proc/loadavg`
- `/proc/meminfo`: `MemAvailable`, `Slab`, `SUnreclaim`, `CmaFree`
- fps: the delta of `rustkvm_video_frames_total` from `/metrics`, divided by elapsed time
- Events:
  - new `dmesg` lines
  - new ` ERROR ` / ` WARN ` lines in `/userdata/rustkvm/log/rustkvm_app.log`, read from a byte offset
  - any change of pid
- Session-leak probe: socket fds of the app, line counts of `/proc/net/udp` and `/proc/net/udp6`, `RssAnon`, `Threads`

## 5. Viewer and session churn

### The viewer

1. Start headless Chromium with:
   - `--remote-debugging-port`
   - `--no-proxy-server`
   - `--autoplay-policy=no-user-gesture-required`
   - `--mute-audio`
   - a fresh `--user-data-dir` per instance

   Use Playwright's bundled Chromium. On the WSL host, Chrome for Testing 148 hung its renderer after Connect.
2. Over CDP:
   1. `PUT /json/new`
   2. `Page.enable`. It must come before the next step, or the injected script never runs.
   3. `Page.addScriptToEvaluateOnNewDocument`, with a script that wraps `RTCPeerConnection` and keeps each instance in `window.__pcs`
   4. `Page.navigate` to the device
   5. `Runtime.evaluate` to click the button labelled Connect / 连接 / Reconnect / 重新连接
3. Poll `__pcs` every 0.5 s until a connection reports `connected`, and record the connect time.
4. Every 10 s, read `getStats`:
   - from `inbound-rtp` video: `framesPerSecond`, frame size, `framesDecoded` / `framesDropped`, `packetsLost` / `packetsReceived`, `freezeCount`, `totalFreezesDuration`, bytes and jitter
   - the candidate-pair RTT
5. After two consecutive samples that are not `connected`, click Reconnect. The frontend does not reconnect on its own yet.
6. Rotate sessions by closing the tab (`/json/close`) and opening a new one every N minutes. Use N = 15 for the baseline and N = 5 to amplify session leaks.

### Churn acceptance

1. Run 10 plain sessions: connect, hold 5 s, close.
2. Run 5 takeovers: client A connects, then client B connects, which triggers `otherSessionConnected`.
3. Record the session-leak probe 3 s after each close, and once more after 60 s idle.

Pass criteria:

- sockets, udp and udp6 return to baseline ±2
- `RssAnon` grows by less than 10 MB, not counting the first session's one-off warm-up
- threads do not grow with the session count
- in the app log, the counts of "Creating new WebRTC session", connected and "Video track attached" match
- no panic, no new ERROR, and no "negotiated no H.264 profile"

## 6. Analysis

- Use the longest stretch with an unchanged pid. Drop the first 0.5 h as warm-up.
- For each series, fit a least-squares slope over the whole stretch and over its second half. Also record the median of the first and of the last 30 min, and the min and max.
- Flag a series as a leak suspect when its second-half slope exceeds:

  | Series | Threshold per hour |
  | --- | --- |
  | RSS, anon, PSS | 2048 KB |
  | threads | 0.5 |
  | fds | 0.5 |
  | maps | 2 |
  | dmabuf maps | 0.5 |
  | SUnreclaim | 2048 KB |

- Also report:
  - fps median and min
  - max temperature
  - new dmesg lines and new app ERROR/WARN lines
  - ops ok/fail with failure categories
  - the leak suspects
- Read the curve's shape, not only the slope.
  - Monotonic linear growth is a leak.
  - A saw-tooth that also falls back points at glibc arena growth. Two ways to tell them apart:
    - diff `/proc/<pid>/smaps` grouped by 64 MB-aligned arena start
    - an A/B run with `MALLOC_ARENA_MAX=2`, and a third arm with `MALLOC_MMAP_THRESHOLD_=1048576`, which disables the dynamic mmap threshold
  - Remove those variables from `S99rustkvm` afterwards.

## 7. Schedule

1. Functional matrix, about 2 min.
2. 15 min idle baseline with no workers.
3. 7 h full load, viewer rotating every 15 min.
4. After fixes, a 3 h re-run with the viewer rotating every 5 min.
5. An 8 h run to settle any remaining drift.

Take a snapshot every 1.5–2 h. Watch the event log for the first 10 minutes and fix harness errors before trusting the run.

## Findings of the October 2026 pass

Fixed in PR #85 (all verified on hardware):

- f_hid kernel BUG after a UDC rebind
- WebRTC sessions never released: a reference cycle through the ICE state handler; 4 UDP sockets and about 11 MB per session
- 12 MB dma-heap leak per screenshot
- macros pre-empting each other
- `restartVideoPipeline` racing the supervisor
- built-in ISO mounted as a disk
- abandoned-upload fd leak
- MQTT enable, tailscale status and the video sleep default

Still open when the pass stopped:

- `get_atx_state` / `getATXState` return Internal error when no ATX extension is attached.
- The frontend does not reconnect after an ICE failure.
- Anonymous memory still drifts in a saw-tooth: about 17 MB/h over the last 4 h of a 5.9 h run, with drops of up to 35 MB.
  - smaps diffs put the growth in two existing per-thread arenas rather than new ones.
  - Jumps are multiples of about 12 MB, the size of one 4K NV12 frame, which points at large transient buffers on the OCR and screenshot path.
  - The `MALLOC_*` A/B runs were not done.
- One `restartVideoPipeline` hit `cma: alloc failed … 12.4 MB` in `VIDIOC_REQBUFS`, with `WARNING at mm/page_alloc.c`, and recovered within 1 s. CMA is 128 MB and the capture buffers take about 87 MB. Options:
  - fewer capture buffers
  - a larger CMA
  - a retry with fewer buffers when `REQBUFS` fails
- `setUsbConfig`, `setUsbDevices` and `setUsbDeviceState` are no-ops.

## Pitfalls

- **WSL disk.** WSL is torn down when the Windows system drive is nearly full (0.2 GB free here), killing cargo and docker builds mid-way. Free space and compact the vhdx before building.
- **node not on PATH.** After a WSL restart, node installed by fnm is missing from PATH; call it by absolute path.
- **pkill / pgrep -f.** A pattern that also appears in the calling command line kills the calling shell too. Resolve pids first, then kill them.
- **One session at a time.** A second client takes over the session, including the harness viewer. Anyone using the web UI during a run invalidates the baseline.
- **USB rebinds on old builds.** UDC rebinds disconnect USB on the controlled host. On builds before PR #85, a SIGTERM after a rebind triggers the f_hidg_read kernel BUG.
- **CMA warnings.** Add `cma: alloc failed` to the dmesg patterns you watch.

## Side effects on the controlled host

A run leaves traces on the controlled host. Record them, so the host can be restored afterwards.

- **Windows left open:** the Run box (Win+R), Task View, a "Insert disk" prompt for the empty CD drive. On builds that mounted the ISO as a disk, a "Format drive" dialog also appeared.
- **Stray input:** random letters typed into whatever has focus. Centre clicks may launch a desktop shortcut.
- **Toggled state:** CapsLock (put back after each LED check).
- **Hotkeys used:** Escape, Backspace, Enter, Win+R, Win+D, Ctrl+N, Ctrl+A.

The run also changes the device:

- **Config:** setters that write back the current value persist implicit defaults (`edid_string`, `tls_mode`). Diff `config.toml` against `config.toml.bak.pre-soak`.
- **Mass storage mode:** left at `cdrom` after ISO mounts. Restore the previous value with `setMassStorageMode`.
- **New files under `/userdata/rustkvm`:** `mqtt-config.toml`, `serialCommandHistory.json`, and `images/netboot.xyz-multiarch.iso`.
