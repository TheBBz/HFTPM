#!/bin/bash

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "🌐 HFTPM - Hetzner Deployment Script"
echo "======================================"
echo ""

check_hcloud() {
    if ! command -v hcloud &> /dev/null; then
        echo "❌ hcloud CLI not installed"
        echo "   Install from: https://github.com/hetznercloud/cli"
        exit 1
    fi
}

check_ssh() {
    if ! command -v ssh &> /dev/null; then
        echo "❌ ssh not installed"
        exit 1
    fi
}

echo "📋 Checking prerequisites..."
check_hcloud
check_ssh
echo "✅ Prerequisites OK"
echo ""

SERVER_NAME="${1:-hfptm-prod}"
SERVER_TYPE="${2:-cx51}"
SERVER_LOCATION="${3:-nbg1}"

echo "🏗 Server Configuration:"
echo "   Name: $SERVER_NAME"
echo "   Type: $SERVER_TYPE"
echo "   Location: $SERVER_LOCATION"
echo ""

echo "🚀 Creating Hetzner server..."
hcloud server create \
    --name "$SERVER_NAME" \
    --type "$SERVER_TYPE" \
    --location "$SERVER_LOCATION" \
    --image ubuntu-22.04 \
    --ssh-key hfptm-deploy \
    --enable-protection \
    --automount \
    --volume-size 100

if [ $? -eq 0 ]; then
    echo "✅ Server created successfully"
else
    echo "❌ Failed to create server"
    exit 1
fi
echo ""

echo "⏳ Waiting for server to be ready (this may take 1-2 minutes)..."
sleep 60

SERVER_IP=$(hcloud server describe "$SERVER_NAME" | grep -oP "IPv4:" | cut -d' ' -f2)
echo "📡 Server IP: $SERVER_IP"
echo ""

echo "📡 Waiting for SSH to be available..."
for i in {1..60}; do
    if ssh -o ConnectTimeout=5 -o StrictHostKeyChecking=no root@$SERVER_IP "echo 'SSH is ready'" &> /dev/null 2>&1; then
        echo "✅ SSH is ready"
        break
    fi
    echo "   Waiting... ($i/60)"
    sleep 2
done
echo ""

echo "📦 Installing dependencies on server..."
ssh root@$SERVER_IP << 'EOF'
set -e

apt update && apt upgrade -y

apt install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    git \
    tmux \
    htop \
    curl \
    wget \
    jq

curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh
source /root/.cargo/env

echo performance > /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

systemctl stop irqbalance || true
systemctl disable irqbalance || true

echo "net.core.default_qdisc=fq" >> /etc/sysctl.conf
echo "net.ipv4.tcp_congestion_control=bbr" >> /etc/sysctl.conf
sysctl -p

echo "* soft nofile 65536" >> /etc/security/limits.conf
echo "* hard nofile 65536" >> /etc/security/limits.conf
sysctl -p

ufw allow 22/tcp
ufw allow 80/tcp
ufw allow 443/tcp
ufw enable

echo "✅ Dependencies installed"
EOF

echo ""

echo "📁 Creating project directory..."
ssh root@$SERVER_IP "mkdir -p /opt/hfptm"
echo "✅ Project directory created"
echo ""

echo "📤 Copying project files..."
tar -czf - . | ssh root@$SERVER_IP "cd /opt/hfptm && tar -xzf -"
echo "✅ Project files copied"
echo ""

echo "🔨 Building HFTPM on server..."
ssh root@$SERVER_IP << 'EOF'
cd /opt/hfptm
cargo install --locked --path .
cargo build --release
echo "✅ Build complete"
EOF

echo ""

echo "👤 Creating service user..."
ssh root@$SERVER_IP << 'EOF'
useradd -m -s /bin/bash hfptm
usermod -aG sudo hfptm
echo "hfptm ALL=(ALL) NOPASSWD: /opt/hfptm/target/release/hfptm" > /etc/sudoers.d/hfptm
chown -R hfptm:hfptm /opt/hfptm
echo "✅ User created"
EOF

echo ""

echo "📝 Installing systemd service..."
ssh root@$SERVER_IP << 'EOF'
cat > /etc/systemd/system/hfptm.service << 'SERVICEFILE'
[Unit]
Description=HFTPM Polymarket Arbitrage Bot
After=network-online.target

[Service]
Type=simple
User=hfptm
WorkingDirectory=/opt/hfptm
ExecStart=/opt/hfptm/target/release/hfptm
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
SERVICEFILE

systemctl daemon-reload
systemctl enable hfptm
systemctl start hfptm

sleep 5
systemctl status hfptm
EOF
echo "✅ Service installed"
echo ""

echo "🔐 Secrets setup reminder..."
echo "⚠️  IMPORTANT: You still need to configure secrets!"
echo ""
echo "1. SSH into server:"
echo "   ssh root@$SERVER_IP"
echo ""
echo "2. Copy secrets template:"
echo "   cd /opt/hfptm"
echo "   cp config/secrets.toml.example config/secrets.toml"
echo ""
echo "3. Edit secrets:"
echo "   nano config/secrets.toml"
echo ""
echo "4. Restart service:"
echo "   sudo systemctl restart hfptm"
echo "5. View logs:"
echo "   sudo journalctl -u hfptm -f"
echo ""

echo "======================================="
echo "✅ Deployment complete!"
echo ""
echo "📡 Server ready at: $SERVER_IP"
echo "🌐 Dashboard available at: http://$SERVER_IP:3000"
echo ""
echo "📚 Post-deployment checklist:"
echo "   [ ] Configure config/secrets.toml"
echo "   [ ] Verify WebSocket connection"
echo "   [ ] Check dashboard metrics"
echo "   [ ] Monitor first few trades"
echo "   [ ] Set up Telegram alerts"
echo "   [ ] Configure backup strategy"
echo ""
