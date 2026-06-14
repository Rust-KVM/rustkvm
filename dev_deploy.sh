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

# 1. Build application
echo "🔨 Building application..."
cargo +stage2 build -Z build-std=std,panic_abort --target aarch64-unknown-linux-gnu -p rustkvm --bin rustkvm_app --release

# 2. Stop existing process gracefully, then clean up.
#    rustkvm_app arms the hardware watchdog and only disarms it during graceful
#    shutdown. A SIGKILL while it still holds /dev/watchdog (e.g. via `fuser -k`
#    below, which holds video0/80/443) would skip the disarm and REBOOT the device.
#    So: SIGTERM, wait for the clean ~1s exit, and only then force-clean leftovers.
echo "📱 Stopping existing process (graceful, watchdog-safe)..."
ssh "${REMOTE_USER}@${REMOTE_HOST}" '
  if pid=$(pgrep -x rustkvm_app); then
    kill -TERM "$pid" 2>/dev/null || true
    for _ in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || break; sleep 0.5; done
    kill -0 "$pid" 2>/dev/null && echo "warning: rustkvm_app still running after 10s"
  fi
  fuser -k /dev/video0 2>/dev/null || true
  fuser -k 80/tcp 2>/dev/null || true
  fuser -k 443/tcp 2>/dev/null || true
  rm -f /userdata/rustkvm/bin/rustkvm_app /userdata/rustkvm/log/rustkvm_app.log
'

# 3. Deploy to device
echo "📦 Deploying to device..."
ssh "${REMOTE_USER}@${REMOTE_HOST}" 'mkdir -p /userdata/rustkvm/bin'
ssh "${REMOTE_USER}@${REMOTE_HOST}" 'mkdir -p /userdata/rustkvm/log'
ssh "${REMOTE_USER}@${REMOTE_HOST}" "cat > /userdata/rustkvm/bin/rustkvm_app" < target/aarch64-unknown-linux-gnu/release/rustkvm_app
ssh "${REMOTE_USER}@${REMOTE_HOST}" 'chmod +x /userdata/rustkvm/bin/rustkvm_app'

# # 4. Start application
# echo "▶️  Starting application..."
# ssh "${REMOTE_USER}@${REMOTE_HOST}" 'nohup setsid /userdata/rustkvm/bin/rustkvm_app </dev/null >>/userdata/rustkvm/log/rustkvm_app.log 2>&1 &'
# ssh "${REMOTE_USER}@${REMOTE_HOST}" 'sleep 2'

# # 5. Show logs
# echo "📋 Showing logs (Press Ctrl+C to exit)..."
# ssh "${REMOTE_USER}@${REMOTE_HOST}" 'tail -f /userdata/rustkvm/log/rustkvm_app.log'
ssh "${REMOTE_USER}@${REMOTE_HOST}"
