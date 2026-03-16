# RustKVM

If you find this project useful, please consider supporting its development.
BTC：`bc1q3pgq8mc7dm9vvygd7aatq4hnt7j596jcjughm3`

---


## 1. Requirements

- Recommended: Ubuntu-24.04 (WSL2 is supported), use an amd64/x86 architecture device.

- Install rustup nightly

- File `buildroot-rk3588-20250527.tar`

  ```bash
  sudo apt update
  sudo apt install pkg-config gcc-arm-linux-gnueabihf clang llvm-dev libclang-dev unzip \
    build-essential zstd \
    device-tree-compiler gperf \
    libnl-3-dev libdbus-1-dev libelf-dev libmpc-dev dwarves \
    bc openssl flex bison libssl-dev python3 python-is-python3 texinfo kmod cmake
  ```



## 2. Using buildroot-rk3588

```bash
mkdir -p buildroot-rk3588-20250527 && tar -xf buildroot-rk3588-20250527.tar -C buildroot-rk3588-20250527
cd buildroot-rk3588
```

- Do not extract `buildroot-rk3588-20250527.tar` on non-Linux systems.



## 3. Build rk3588-buildkit

```bash
# in buildroot-rk3588 dir
./build.sh rk3588.mk
```

Obtain **$PWD/buildroot/output/rockchip_rk3588/host/**

### Install buildkit

```bash
sudo mkdir -p /opt/rk3588-buildkit
sudo cp -r $PWD/buildroot/output/rockchip_rk3588/host/* /opt/rk3588-buildkit/
```



## 4. Reference Directory

```text
rustkvm
  ├── rustkvm
  └── rust
```

## 5. Clone the project

```bash
git clone https://github.com/Rust-KVM/rustkvm.git
cd rustkvm
```



## 6. Configure Rust

```bash
git clone https://github.com/rust-lang/rust
cd rust
touch bootstrap.toml
```

Add the following to `bootstrap.toml`:

```toml
[build]
target = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]

[target.aarch64-unknown-linux-gnu]
cc = "/opt/rk3588-buildkit/bin/aarch64-buildroot-linux-gnu-gcc"
# Absolute path
```

```bash
# in rust dir
./x.py build --stage 2 --host x86_64-unknown-linux-gnu --target aarch64-unknown-linux-gnu
rustup toolchain link stage2 build/x86_64-unknown-linux-gnu/stage2
```
See the [platform-support documentation](https://doc.rust-lang.org/nightly/rustc/platform-support/armv7-unknown-linux-uclibceabihf.html) for more details.

## 7. Build the Project

```bash
cargo build -Z build-std --target aarch64-unknown-linux-gnu -p rustkvm
```
