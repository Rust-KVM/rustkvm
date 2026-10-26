# 稳定性压测与复现记录（2026-10-25 ~ 26）

本文是 2026 年 10 月那轮真机压测的中文记录，重点是**每个问题怎么复现、根因、修复提交和验证方式**。测试步骤、负载 worker、采样项、判据和注意事项以 [soak-testing.md](soak-testing.md) 为准，这里不再重复。

测试脚本按项目规则是一次性的，不进仓库。

## 结论

- 功能：140 个 JSON-RPC 方法、18 个 MCP 工具逐项调用。修复后 138 通过 / 2 失败 / 24 跳过。2 个失败是没接 ATX 扩展时 `getATXState` 和 MCP `get_atx_state` 报错，属于现状，待改为友好状态。
- 稳定性：共发现并修复 12 个问题，全部合入 dev（[PR #85](https://github.com/Rust-KVM/rustkvm/pull/85)，merge `47dde28`），其中 1 个会让内核崩溃重启，3 个是持续泄露（会话 socket + 内存、截图 dma-buf、放弃上传的 fd）。
- 修复后 3 h 复测：216 万次操作，失败率 0.078 %（几乎全是无 ATX 扩展和脚本主动取消宏），0 次内核 BUG，0 条 app ERROR，帧率中位数 60，最高 51.8 °C；fd、线程、dma-buf、slab 都走平。
- 修复后长测（5.9 h）：fd、线程、dma-buf 不增长；匿名内存仍有锯齿式波动，判断为 glibc arena 行为而非逐次泄露，分配器 A/B 对照没有做，见[残余内存锯齿](#残余内存锯齿)。

## 环境

| 角色 | 说明 |
| --- | --- |
| 设备 | NanoPC-T6（RK3588），`192.168.1.6`，HDMI 输入 3840x2160@60 |
| 被控机 | 一台 Windows 主机，接设备的 HDMI 和 USB。本轮允许随意发送键鼠 |
| 测试机 | WSL2 工作台（构建、部署、跑脚本），Mac（浏览器观看与网页审查） |
| 版本 | 起点 dev `07e2c0b`（含 OCR #83），修复分支 `claude/project-thread-3za6p9`，终点 dev `47dde28` |

构建部署按 [soak-testing.md 第 1 节](soak-testing.md#1-build-and-deploy)：SIGTERM 停止后等它退出，**不要用 `kill -9`**，看门狗会重启设备。

测试前把 `/userdata/rustkvm/config.toml` 备份为 `config.toml.bak.pre-soak`，测完按备份恢复。

## 方法

构建部署、功能矩阵、各 worker 的频率、采样命令、泄露判据、观看器和会话 churn 验收，见 [soak-testing.md](soak-testing.md) 第 1 ~ 7 节。下面只补充网页界面审查。

### 网页界面审查

在 Mac 上用 Playwright 驱动 Chrome 走完每个页面和设置页：浅色/深色、中文/English、1440/1024/390 px 宽度，加 axe 无障碍扫描、键盘焦点顺序、接管、OCR、终端，以及 3 min 页面内存曲线。新前端没部署前，用一个本地代理提供本地构建的 `dist/`，其余请求（含 WebSocket）转发到设备。结果：没有页面报错，39 个可用性和一致性问题（9 个严重），修复在 [PR #84](https://github.com/Rust-KVM/rustkvm/pull/84)，还在等设备上的重连复测。

## 时间线

| 时间（UTC） | 版本 | 内容 |
| --- | --- | --- |
| 10-25 17:10 | `07e2c0b` | 功能矩阵首轮：134 通过 / 6 失败 / 24 跳过 |
| 10-25 17:31 | `07e2c0b` | 复现 USB 重绑后的内核崩溃，设备重启 |
| 10-25 19:09 | 中间版本 | LED 读线程不在 runtime 里，app 崩溃，看门狗重启（修复过程中引入的回归，已修并补测试） |
| 10-25 19:28 ~ 10-26 02:30 | `3fa603c` | 7 h 压测（作为修复前数据）：467 万次操作，主程序 0 次重启，0 内核报错，60 fps |
| 10-26 03:00 ~ 04:10 | — | WSL 反复崩溃，原因是 Windows C 盘只剩 0.2 GB，压缩虚拟磁盘后恢复 |
| 10-26 04:30 | `7d394aa` | 会话 churn 验收：50 次会话后 socket 回到基线 |
| 10-26 04:48 ~ 07:50 | `7d394aa` | 3 h 复测，通过 |
| 10-26 07:51 | `47dde28` | PR #85 合入 dev |
| 10-26 07:53 ~ 约 12:00 | `47dde28` | 8 h 长测，跑到约 4 h 停止 |

## 问题清单

提交都在 PR #85 里。

### 1. 开关 USB 模拟后内核崩溃重启

- **复现**：`setUsbEmulationState(false)`，再 `setUsbEmulationState(true)`，然后给 app 发 SIGTERM（或让主机发一次键盘 LED 变化）。设备打印 `kernel BUG at lib/list_debug.c:59`，调用栈在 `f_hidg_read`，随即重启。崩溃信息在重启后的 pstore 或上一次的 `dmesg` 里。
- **根因**：键盘 LED 读线程一直阻塞 `read` 在 `/dev/hidg0` 上。UDC 重绑时 6.1 内核的 f_hid 在 `hidg_bind()` 里重新初始化读写等待队列，睡着的读线程醒来后从已重置的链表摘除自己，链表损坏。
- **修复**：`5fa034b`。读线程改为 `O_NONBLOCK` fd + 100 ms 轮询；`Hid::suspend_io()` / `resume_io()` 包住 unbind/bind；`setUsbEmulationState` 幂等（原来对已绑定的 UDC 再开启会报 EBUSY）。
- **连带的两个问题**：`3fa603c` 修复 LED 回调在非 tokio 线程里 spawn 导致 panic（`panic = "abort"` 下 app 直接退出）；`49cb544` 让 `resume_io` 等 mdev 重建 `/dev/hidg0`（最多 5 s），不然重绑后 LED 状态不再更新。`app/tests/hid_led_reader.rs` 用 FIFO 驱动读线程，修复前失败。
- **验证**：重绑后 SIGTERM 重启 ×3 不崩；3 h 复测中 6 次重绑 0 次 BUG；重绑后不重启 app，10 s 内主机按 CapsLock，`getKeyboardLedState` 跟着变。

### 2. WebRTC 会话结束后泄露 socket 和内存

- **复现**：建立并关闭观看会话 N 次（刷新页面、断网重连、第二个客户端接管都算）。每个会话结束后 `ss -uanp` 里 app 的 UDP socket 多 4 个（每个本地地址一个），`RssAnon` 多约 11 MB，永不回收。压测中每 15 min 重连一次时 fd 涨 17 ~ 38 个/h，anon 涨 43 ~ 107 MB/h。
- **根因**（两层）：
  1. 会话在 ICE failed/closed 时被移除，但没调用 `pc.close()`；WebSocket 关闭路径会 close，可通常 ICE 回调先把会话删了，它就找不到要关的连接。修复 `ddd0b80`：`AppState::remove_session` 统一在独立任务里 close 被移除会话的 peer connection。
  2. close 之后仍不释放：ICE 状态回调强持有 `RTCRtpSender` 和本地 track，它们持有 DTLS/ICE transport，transport 持有 ICE agent，ICE agent 又持有这个回调，形成引用环。修复 `7d394aa`：回调里改持 `Weak`。
- **验证**：50 次会话（含接管）后 socket / udp / udp6 回到基线，最后 30 次内存净变化 −1.2 MB；3 h 复测 fd 斜率 −0.2/h。

### 3. 每张截图泄露一块 12 MB dma-buf

- **复现**：记录 `grep -c dmabuf /proc/$PID/maps`，调用 `GET /device/screenshot` 20 次（或 MCP `screenshot`），映射数多出 18 ~ 22 个，每个 12443648 字节（3840x2160 NV12 一帧），不回收。正常使用 1 h 累积约 600 个、8 GB 映射，持续截图约 20 min 会耗尽内存。
- **根因**：每次截图新建 `appsrc ! videoconvert ! mppjpegenc ! appsink` 再置 NULL；mppjpegenc 把输入帧拷进 dma-heap 缓冲，管线销毁时不释放。
- **修复**：`3a0f12d`。JPEG 管线建一次复用，只在 caps 变化或编码失败时重建。
- **验证**：截图 120 次映射数不变；压测中 1.2 万多张截图，dma-buf 走平。

### 4. 新宏会打断正在执行的宏

- **复现**：两个客户端同时 `typeText` 不同的 200 个字符，先发的那个返回 `keyboard macro was cancelled or failed`，主机上只出现一部分文字。网页粘贴和 MCP `type_text` 同时用也会这样；关掉网页数据通道还会取消经 HTTP/MCP 发起的宏。修复前 7 h 压测里有 1885 次这类失败，开头 10 min 就有 42 次。
- **根因**：每个宏开始前都先取消当前在跑的宏。
- **修复**：`f57ae4e`。改成 FIFO 排队；只有 `cancelKeyboardMacro` 会取消（当前的加排队的）；数据通道关闭只取消它自己发起的宏。
- **验证**：两个并发 `typeText` 都成功，文字完整、不交错；1000 字符的 `typeText` 中途取消，`getKeyDownState` 为空，没有卡住的键。

### 5. 手动重启视频管线和自动恢复撞车

- **复现**：间隔 3 ~ 8 s 连续调用 `restartVideoPipeline` 20 次，同时每秒截图一次。约 1/8 的概率日志里同时出现两次 `Creating mpph264enc`、`Audio pipeline failed to start`、hdmirx `queue busy`，之后约 10 s 没有画面（截图 503，OCR 报 `no video frame`）。
- **根因**：`restartVideoPipeline` 在 stop 和 start 之间没有管线，看守任务恰好这时 tick，以为管线挂了也去重建，同时建出两个编码器。
- **修复**：`9ad9dae`。重建锁 `RESTART_LOCK` 覆盖 stop + start，看守任务用 `try_lock`，别人在重建时跳过这一轮。
- **验证**：重启 ×20，同时 117 张截图、55 次 `find_text`，0 失败，没有并发创建编码器的日志。

### 6. 管线重建期间截图和 OCR 直接失败

- **复现**：调用 `restartVideoPipeline` 的同时调用 `find_text`，返回 `Pipeline not initialized` 或 `Video pipeline stopped`。
- **修复**：`e3b166a`。`capture_raw_frame` 遇到重建中最多等 5 s；管线在跑但没有帧的情况仍然 2 s 后失败。

### 7. 放弃的上传泄露文件句柄

- **复现**：调用 `startStorageFileUpload` 但不发数据，重复 3 次，再 `deleteStorageFile("<name>.incomplete")`。`ls -l /proc/$PID/fd | grep incomplete` 仍能看到打开的句柄，删除还会报文件不存在。
- **修复**：`93c1be2`。同一文件的新上传替换旧条目；删除文件时丢弃它的待传条目；HTTP 上传出错时收尾（保留半截文件，可续传）。

### 8. 内置 ISO 按磁盘挂载

- **复现**：`mountBuiltInImage`（netboot.xyz），Windows 上出现一个未格式化的可移动磁盘，并弹出格式化对话框。
- **修复**：`9fb78ff`。`.iso` 以 CD-ROM 方式挂载（lun `cdrom=1 ro=1`），`getMassStorageMode` 返回 cdrom。

### 9. MQTT 开机关闭时无法开启，状态任务越积越多

- **复现**：开机时 MQTT 处于关闭状态，调用 `setMqttSettings`（即使原样写回）报 `MQTT manager not initialized`。另外每次重启 MQTT 都多出一个周期状态发布任务。
- **修复**：`c9f230d`。`restart_mqtt` 负责创建 manager；保存任务句柄，重启时 abort 旧任务。

### 10. 没装 tailscale 时 `getTailscaleStatus` 报内部错误

- **复现**：在没有 tailscale 二进制的精简镜像上调用 `getTailscaleStatus`。
- **修复**：`c9f230d`。找不到二进制时返回 `online: false`。

### 11. 视频休眠默认值原样写回就会被打开

- **复现**：先 `getVideoSleepMode`，再把结果原样传给 `setVideoSleepMode`，`enabled` 从 false 变成 true。
- **根因**：默认值是 `enabled=false, duration=60`，而 setter 按 `duration >= 0` 推导 `enabled`。
- **修复**：`49cb544`。默认 duration 改为 −1（关闭）。

### 12. 重复开启 USB 模拟报 EBUSY

- **复现**：UDC 已绑定时再调用 `setUsbEmulationState(true)`。
- **修复**：`5fa034b`（随问题 1），setter 幂等。

## 残余内存锯齿

修复后的长测里匿名内存仍有锯齿式波动，最新数据（5.9 h 那轮的最后 4 h 约 17 MB/h，回落最多 35 MB）和 smaps 分析见 [soak-testing.md 的 Findings](soak-testing.md#findings-of-the-october-2026-pass)。要点：

- fd、线程、dma-buf 都没有增长，涨的只有匿名内存。
- RSS 里约 670 MB 是首次 OCR 加载的 NPU 模型，一次性常驻。
- 跳变有涨有落、常常成对出现，和重绑、管线重启、挂载、观看器重连都对不上；幅度多是 10 ~ 12 MB 的整数倍，接近一帧 4K NV12（12.4 MB）。
- smaps 差分显示增长落在已有的两个 per-thread arena 里，不是新开 arena。

判断：OCR/截图路径的大块临时分配留在 glibc arena 里没有还给系统，属于分配器行为而非逐次操作泄露。分配器 A/B 对照还没做，仍是待定项。

## 后续

- 分配器对照，每组 1 h、负载相同（OCR + 截图 + 观看器），记录 anon 斜率、峰值和 CPU：A 不改；B `MALLOC_ARENA_MAX=2`；C `MALLOC_MMAP_THRESHOLD_=1048576`（关闭动态阈值，大块分配始终走 mmap）。环境变量只在测试期间临时加进 `S99rustkvm`，测完恢复。若 C 明显压住增长，在代码里用 `mallopt` 固定阈值或复用帧缓冲作为正式修复。
- 其余未解决项（无 ATX 扩展时报错、ICE 失败后前端不重连、`restartVideoPipeline` 偶发 CMA 分配失败、几个 USB 配置 setter 是空操作）见 [soak-testing.md](soak-testing.md#findings-of-the-october-2026-pass)。

## 注意事项

测试中的坑（WSL 磁盘、单会话接管、旧版本重绑崩溃、CMA 告警）以及对被控机和设备留下的改动、如何还原，见 [soak-testing.md](soak-testing.md#pitfalls)。
