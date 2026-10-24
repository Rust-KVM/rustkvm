<div align="center">

<img src="docs/assets/social-preview.png" alt="RustKVM：基于 RK3588、纯 Rust 实现的开源 IP-KVM" width="820">

# RustKVM

**基于瑞芯微 RK3588、纯 Rust 实现的开源 KVM-over-IP（IP-KVM）。**
在浏览器里以 BIOS 级权限远程控制任意电脑：硬件编码视频经 WebRTC 低延迟传输，
USB 键鼠模拟、虚拟光驱 / U 盘、ATX 电源控制，并内置 MCP 接口，让 AI 智能体也能直接操作目标机器。

[![CI](https://github.com/Rust-KVM/rustkvm/actions/workflows/ci.yml/badge.svg?branch=dev)](https://github.com/Rust-KVM/rustkvm/actions/workflows/ci.yml)
[![Web CI](https://github.com/Rust-KVM/rustkvm/actions/workflows/web.yml/badge.svg?branch=dev)](https://github.com/Rust-KVM/rustkvm/actions/workflows/web.yml)
[![License: GPL-2.0-only](https://img.shields.io/badge/license-GPL--2.0--only-blue.svg)](LICENSE)

[English](README.md) · [功能](#功能) · [界面](#界面概览) · [快速开始](#快速开始) · [构建](#从源码构建) · [API](#api-概览)

</div>

---

## 这是什么

RustKVM 把一块 RK3588 开发板变成 **IP-KVM**：将它的 HDMI 输入和 USB OTG 口接到目标电脑，
即可在浏览器中获得目标机的画面、键盘、鼠标、存储和电源按钮，从开机固件阶段就可用，目标机无需安装任何软件。

它沿用 [JetKVM](https://github.com/jetkvm/kvm) 的设备端思路，用 Rust 重写并移植到 RK3588，
由 SoC 内置的 **Rockchip MPP** 硬件编码器完成视频编码。后端与浏览器前端（Leptos 编译为 WebAssembly）
都是 Rust，共用同一个协议 crate。

**适合谁用**

- 想要 BIOS 级远程访问、又不想依赖厂商 BMC 的 **家庭实验室和服务器运维者**
- 需要远程看启动日志、刷镜像、给板子断电重启的 **硬件与固件开发者**
- **自动化与 AI 智能体开发者**：所有设备操作都是 JSON-RPC 方法，可经 WebRTC、HTTP、MQTT 或 MCP 调用

> [!NOTE]
> RustKVM 处于活跃开发阶段（工作区版本 `0.1.0`），尚无正式发布版本，请按下文从源码构建。

## 功能

- **硬件加速视频**：GStreamer + MPP（`mpph264enc` / `mpph265enc`）H.264 / H.265 编码，默认 60 fps，支持 VBR / CBR / AVBR
- **零拷贝推流**：编码帧以 `bytes::Bytes` 直接进入 WebRTC RTP，无逐帧内存拷贝
- **WebRTC 低延迟**：Opus 音频、Socket.IO 信令、视频管线崩溃后自动退避重启
- **USB HID**：键盘 + 绝对 / 相对鼠标，二进制 HID-RPC 走可靠与不可靠两类数据通道
- **键盘布局**（英 / 美、德、法、西、意、日）、锁定键指示灯、Ctrl+Alt+Del、文本输入、键盘宏、鼠标防休眠
- **虚拟介质**：将 ISO / 磁盘镜像以 USB 光驱或磁盘挂载，来源可以是设备存储、HTTP URL、浏览器上传或内置 [netboot.xyz](https://netboot.xyz)
- **电源控制**：ATX 开机 / 复位键与电源 / 硬盘灯状态（GPIO），DC 电源扩展及来电恢复策略，网络唤醒（WoL）
- **串口控制台与 Web 终端**、EDID 设置、屏幕截图（JPEG）、OCR 识别屏幕文字
- **JSON-RPC 2.0**：130+ 个方法，与 Web UI 使用同一套
- **MCP 服务**（`POST /mcp`）：截图、打字、组合键、鼠标、电源与通用 `rpc_call`，Claude 等 MCP 客户端可直接驱动目标机
- **API Token**（`Authorization: Bearer rkvm_…`，仅存 SHA-256 哈希）
- **MQTT + Home Assistant 自动发现**、**Prometheus 指标**（`/metrics`）、健康检查（`/device/health`）、OpenAPI（`/scalar-ui`）
- **默认 HTTPS**（启动时生成自签名证书，也可使用自定义证书）、密码 / 免密模式、首次启动向导、登录指数退避限流
- **mDNS**（`rustkvm.local`）、DHCP / 静态 IP、Tailscale、可选云中继（OIDC）、故障安全模式

## 界面概览

| 区域 | 内容 |
| --- | --- |
| **视频画面** | 目标机实时画面，点击即接管键鼠，可切换绝对 / 相对指针 |
| **工具栏** | 全屏、Ctrl+Alt+Del、画质增减、设备音频、OCR 复制屏幕文字、串口终端、重启、设置 |
| **状态指示** | 目标机上报的 Caps / Num / Scroll Lock 指示灯 |
| **设置抽屉** | 功能开关、虚拟介质、USB 设备、视频编码与休眠、EDID、键盘布局、网络、WoL、MQTT、屏幕旋转与背光、SSH 公钥、密码、日志级别、关于 |
| **高级设置** | ATX / DC 电源、扩展板、键盘宏、串口控制台、TLS、Tailscale、云、开发者模式、恢复出厂 |

## 与同类项目对比

| | **RustKVM** | [JetKVM](https://github.com/jetkvm/kvm) | [PiKVM](https://github.com/pikvm/pikvm) |
| --- | --- | --- | --- |
| 硬件 | 任意带 HDMI-RX 的 RK3588 板卡 | JetKVM 设备 | 树莓派 + 采集卡 |
| 设备端语言 | Rust | Go | Python |
| Web 界面 | Rust / WebAssembly（Leptos） | React | JavaScript |
| 许可证 | GPL-2.0-only | GPL-2.0 | GPL-3.0 |

RustKVM 的侧重点：RK3588 的 8 核算力与 H.264 / H.265 硬件编码；后端、前端与协议全部使用 Rust；
所有操作都是 JSON-RPC 方法，并通过 HTTP、MQTT 和内置 MCP 接口对外开放。

## 快速开始

完整步骤（刷机、接线、首次登录）见 [docs/getting-started.md](docs/getting-started.md)。

1. 把固件镜像写入 microSD 卡（[docs/quickstart.md](docs/quickstart.md)），可用 Release 下载的，也可自己构建（见下节）；固件已自带 RustKVM
2. 浏览器打开 `https://<设备IP>/` 或 `https://rustkvm.local/`，接受自签名证书
3. 完成设置向导，点击画面即可开始控制
4. 部署自己构建的程序（可选）：在 设置 → 安全 → SSH 访问 填入公钥，在 设置 → 系统 → 开发者 打开开发者模式，然后执行 `./dev_deploy.sh -r <设备IP>`（没有 stage2 工具链时自动在 devkit 容器里编译）

## 从源码构建

**最快路径：任意装了 Docker 的 Linux 主机（含 WSL2）**，无需登录 GitHub 或 `docker login`，镜像都是公开的：

```bash
git clone https://github.com/Rust-KVM/rustkvm && cd rustkvm
scripts/devkit.sh run scripts/build-app.sh   # → out/app/rustkvm_app-aarch64-unknown-linux-gnu，约 7 分钟
scripts/devkit.sh firmware                   # → out/firmware/ SD 卡与 eMMC 镜像，首次需约 50 GB 空闲磁盘
```

版本固定、磁盘占用、代理与固件构建见 [docs/building.md](docs/building.md)；改主机名、界面名称、USB 标识和固件版本见 [docs/customizing.md](docs/customizing.md)。WSL2 下 Docker 和 Linux 文件都在 C 盘的虚拟磁盘里，可用空间以 C 盘为准。

不用 Docker 时的手动步骤：

```bash
# 1. 前端（任意主机，stable Rust）
rustup target add wasm32-unknown-unknown && cargo install trunk
cd crates/rkvm-web && trunk build --release && cd -

# 2. x86_64 主机构建（开发与测试，与 CI 相同；主机上没有 MPP，不推流）
sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
cargo +nightly test --workspace

# 3. aarch64 设备构建（需 /opt/rk3588-buildkit 工具链；stage2 是参考工作站自建的，也可换成 +nightly-2026-06-14）
cargo +stage2 build -Z build-std=std,panic_abort \
  --target aarch64-unknown-linux-gnu -p rustkvm --bin rustkvm_app --release
```

交叉编译环境的完整搭建步骤见 [英文 README](README.md#3-device-build-aarch64--rk3588)。

## API 概览

除公开接口外，均需登录 Cookie 或 `Authorization: Bearer <API token>`。详见 [`docs/api.md`](docs/api.md)。

```bash
curl -k https://rustkvm.local/device/rpc \
  -H "Authorization: Bearer $RUSTKVM_TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}'

# 将设备接入 Claude Code 等 MCP 客户端
claude mcp add --transport http rustkvm https://rustkvm.local/mcp \
  --header "Authorization: Bearer $RUSTKVM_TOKEN"
```

## 参与贡献

欢迎提交 Issue 与 PR，请先阅读 [`CONTRIBUTING.md`](CONTRIBUTING.md) 与 [行为准则](CODE_OF_CONDUCT.md)。
安全问题请按 [`SECURITY.md`](SECURITY.md) 私下报告。

## 许可证

[GPL-2.0-only](LICENSE)
