#!/bin/bash

# RustKVM Development Deployment Script

set -e  # Exit on error

echo "🚀 Starting RustKVM deployment..."

# Args: -r/--remote <host> (required), -u/--user <user> (optional, default: root)
REMOTE_USER="root"
while [[ $# -gt 0 ]]; do
  case $1 in
    -r|--remote)
      REMOTE_HOST="$2"
      shift 2
      ;;
    -u|--user)
      REMOTE_USER="$2"
      shift 2
      ;;
    --help)
      echo "Usage: $0 -r <remote_ip> [-u <remote_user>]"
      echo "Example: $0 -r 192.168.0.17"
      exit 0
      ;;
    *)
      echo "Unknown option: $1"
      echo "Usage: $0 -r <remote_ip> [-u <remote_user>]"
      exit 1
      ;;
  esac
done

if [ -z "${REMOTE_HOST}" ]; then
  echo "Error: remote host is required. Usage: $0 -r <remote_ip> [-u <remote_user>]"
  exit 1
fi

# 1. Build application: the self-built stage2 toolchain if installed, else the Docker devkit
echo "🔨 Building application..."
if rustup toolchain list 2>/dev/null | grep -q '^stage2'; then
  cargo +stage2 build -Z build-std=std,panic_abort --target aarch64-unknown-linux-gnu -p rustkvm --bin rustkvm_app --release
  APP_BIN=target/aarch64-unknown-linux-gnu/release/rustkvm_app
else
  "$(dirname "$0")/scripts/devkit.sh" run scripts/build-app.sh
  APP_BIN=out/app/rustkvm_app-aarch64-unknown-linux-gnu
fi

# 2. Stop the app through its init script: SIGTERM and a wait for the clean
#    exit. rustkvm_app arms the hardware watchdog and only disarms it on a
#    graceful shutdown, so it must never be killed with SIGKILL or `fuser -k`
#    (that reboots the device).
echo "📱 Stopping existing process (graceful, watchdog-safe)..."
ssh "${REMOTE_USER}@${REMOTE_HOST}" '/etc/init.d/S99rustkvm stop'

# 3. Deploy to device. Written beside the target and renamed, so a failed copy
#    never leaves a truncated binary behind.
echo "📦 Deploying to device..."
ssh "${REMOTE_USER}@${REMOTE_HOST}" 'mkdir -p /userdata/rustkvm/bin /userdata/rustkvm/log'
ssh "${REMOTE_USER}@${REMOTE_HOST}" 'cat > /userdata/rustkvm/bin/rustkvm_app.tmp' < "${APP_BIN}"
ssh "${REMOTE_USER}@${REMOTE_HOST}" 'chmod +x /userdata/rustkvm/bin/rustkvm_app.tmp && mv /userdata/rustkvm/bin/rustkvm_app.tmp /userdata/rustkvm/bin/rustkvm_app && sync'

# 4. Start application
echo "▶️  Starting application..."
ssh "${REMOTE_USER}@${REMOTE_HOST}" '/etc/init.d/S99rustkvm start'
echo "✅ Deployed. Log: ssh ${REMOTE_USER}@${REMOTE_HOST} tail -f /userdata/rustkvm/log/rustkvm_app.log"
