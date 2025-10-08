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
cargo build -Z build-std --target aarch64-unknown-linux-gnu -p rustkvm --bin rustkvm_app --release

# 2. Stop existing processes and cleanup
echo "📱 Stopping existing processes..."
ssh "${REMOTE_USER}@${REMOTE_HOST}" 'killall rustkvm_app || true'
ssh "${REMOTE_USER}@${REMOTE_HOST}" 'fuser -k /dev/video0 || true'
ssh "${REMOTE_USER}@${REMOTE_HOST}" 'fuser -k 80/tcp || true'
ssh "${REMOTE_USER}@${REMOTE_HOST}" 'fuser -k 443/tcp || true'
ssh "${REMOTE_USER}@${REMOTE_HOST}" 'rm -f /userdata/rustkvm/bin/rustkvm_app /userdata/rustkvm/log/rustkvm_app.log'

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
