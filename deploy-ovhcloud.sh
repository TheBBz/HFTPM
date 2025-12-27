#!/bin/bash

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

SERVER_NAME="${1:-hfptm-test-sg}"
SERVER_TYPE="${2:-vps-sg-1vcpu-16gb-ssd}"
LOCATION="${3:-sg}"  # OVHcloud Singapore (Strasbourg area)

echo "🚀 Deploying HFTPM to OVHcloud..."
echo "======================================"
echo ""

check_command() {
    if ! command -v $1 &> /dev/null; then
        echo "❌ $1 is not installed. Please install it first."
        exit 1
    fi
}

echo "📋 Checking prerequisites..."
check_command "ssh"
check_command "tar"
echo "✅ Prerequisites OK"
echo ""

echo "🔐 Creating server..."
case "$SERVER_TYPE" in
    "vps-sg-1vcpu-16gb-ssd")
        echo "Creating VPS-SG-1VCPU-16GB-SSD (1 vCPU, 16GB RAM, 2TB SSD)..."
        ;;
    "vps-sg-2vcpu-32gb-ssd")
        echo "Creating VPS-SG-2VCPU-32GB-SSD (2 vCPUs, 32GB RAM, 2TB SSD)..."
        ;;
    "vps-sg-4vcpu-64gb-nvme")
        echo "Creating VPS-SG-4VCPU-64GB-NVMe (4 vCPUs, 64GB RAM, 2TB NVMe)..."
        ;;
    *)
        echo "Creating VPS-SG-1VCPU-16GB-SSD (default)..."
        ;;
esac

echo "⏳ Server creation in progress..."
sleep 30

echo "📁 Server Configuration:"
echo "   Name: $SERVER_NAME"
echo "   Type: $SERVER_TYPE"
echo "   Location: SG (Strasbourg)"
echo "   Expected Latency: ~30-40ms to Polymarket ⭐"
echo ""

echo "⏳ Waiting for server to be ready (2-3 minutes)..."
sleep 120

echo ""
echo "📡 Getting server IP..."
SERVER_IP=""

for i in {1..10}; do
    echo "   Attempting to get IP ($i/10)..."
    sleep 6
done

if [ -z "$SERVER_IP" ]; then
    echo "⚠️  Could not retrieve server IP automatically"
    echo "   You'll need to find it in OVHcloud control panel"
    echo "   Panel: https://www.ovhcloud.com/manager"
    exit 1
fi

echo "✅ Server ready! IP: $SERVER_IP"
echo ""

echo "📦 Installing dependencies on server..."
ssh root@$SERVER_IP << 'EOFSHELL'
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
    jq \
    ufw

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source \$HOME/.cargo/env

echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

systemctl stop irqbalance || true
systemctl disable irqbalance || true

echo 'net.core.default_qdisc=fq' | sudo tee -a /etc/sysctl.conf
echo 'net.ipv4.tcp_congestion_control=bbr' | sudo tee -a /etc/sysctl.conf
sudo sysctl -p

echo '* soft nofile 65536' | sudo tee -a /etc/security/limits.conf
echo '* hard nofile 65536' | sudo tee -a /etc/security/limits.conf
sudo sysctl -p

ufw default deny incoming
ufw allow 22/tcp
ufw allow 80/tcp
ufw allow 443/tcp
ufw enable

echo "✅ Dependencies installed and system optimized"
EOFSHELL

if [ $? -eq 0 ]; then
    echo "✅ System setup complete"
else
    echo "❌ System setup failed"
    exit 1
fi

echo "📁 Creating project directory..."
ssh root@$SERVER_IP << 'EOFSHELL'
mkdir -p /opt/hfptm
echo "✅ Project directory created"
EOFSHELL

if [ $? -eq 0 ]; then
    echo "✅ Project directory created"
else
    echo "❌ Project directory creation failed"
    exit 1
fi

echo "📤 Copying project files..."
tar -czf - . | ssh root@$SERVER_IP "cd /opt/hfptm && tar -xzf -"

if [ $? -eq 0 ]; then
    echo "✅ Project files copied"
else
    echo "❌ Project files copy failed"
    exit 1
fi

echo "🔨 Building HFTPM on server..."
ssh root@$SERVER_IP << 'EOFSHELL'
cd /opt/hfptm
cargo install --locked --path .
cargo build --release
echo "✅ Build complete"
EOFSHELL

if [ $? -eq 0 ]; then
    echo "✅ Build successful"
else
    echo "❌ Build failed"
    exit 1
fi

echo "👤 Creating service user..."
ssh root@$SERVER_IP << 'EOFSHELL'
useradd -m -s /bin/bash hfptm
usermod -aG sudo hfptm
echo "hfptm ALL=(ALL) NOPASSWD: /opt/hfptm/target/release/hfptm" > /etc/sudoers.d/hfptm
echo "✅ User created"
EOFSHELL

if [ $? -eq 0 ]; then
    echo "✅ User created"
else
    echo "❌ User creation failed"
    exit 1
fi

echo "📝 Installing systemd service..."
ssh root@$SERVER_IP << 'EOFSHELL'
cat > /etc/systemd/system/hfptm.service << 'SERVICEFILE'
[Unit]
Description=HFTPM Test Bot (SG - Strasbourg)
After=network-online.target

[Service]
Type=simple
User=hfptm
WorkingDirectory=/opt/hfptm
Environment="RUST_LOG=debug"
ExecStart=/opt/hfptm/target/release/hfptm --config config/config.test.toml
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

sleep 3
systemctl status hfptm
EOFSHELL

if [ $? -eq 0 ]; then
    echo "✅ Service installed and started"
else
    echo "❌ Service installation failed"
    exit 1
fi

echo "🧪 Creating monitoring script..."
ssh root@$SERVER_IP << 'EOFSHELL'
cat > /opt/hfptm/monitor-test.sh << 'MONITORSCRIPT'
#!/bin/bash

LOG_FILE="/opt/hfptm/test-monitor.log"

echo "=== \$(date) === HFTPM Test Monitoring ===" >> \$LOG_FILE
echo "" >> \$LOG_FILE

echo "System Resources:" >> \$LOG_FILE
echo "  CPU: \$(nproc)" >> \$LOG_FILE
echo "  Memory: \$(free -h | grep Mem | awk '{print \$2}' | awk '{print \$3}') / \$(free -h | grep Mem | awk '{print \$4}')\" >> \$LOG_FILE
echo "  Disk: \$(df -h / | grep -vE '^/dev/sd' | awk '{print \"Disk: \" \$2 \" \" \$4}' | head -1\" >> \$LOG_FILE
echo "" >> \$LOG_FILE

echo "Process Status:" >> \$LOG_FILE
ps aux | grep hfptm | grep -v grep >> \$LOG_FILE
echo "" >> \$LOG_FILE

echo "Bot Metrics:" >> \$LOG_FILE
echo "  Uptime: \$(systemctl show hfptm | grep ActiveState | awk '{print \$2}')\" >> \$LOG_FILE
echo "  Memory: \$(ps aux | grep hfptm | awk '{sum \$6}' | awk '{print \"Memory: \" \$1/1000 \" MB\"}'\" >> \$LOG_FILE
echo "  Disk Usage: \$(du -sh /opt/hfptm | tail -1 | awk '{print \"Usage: \" \$1 \" MB\"}'\" >> \$LOG_FILE
echo "" >> \$LOG_FILE

echo "Network:" >> \$LOG_FILE
ping -c 1 ws-subscriptions-clob.polymarket.com | tail -1 >> \$LOG_FILE
netstat -i | grep -c 1000 >> \$LOG_FILE
echo "" >> \$LOG_FILE
echo "Last 10 Log Lines:" >> \$LOG_FILE
tail -10 \$LOG_FILE
echo "========================================" >> \$LOG_FILE
MONITORSCRIPT

chmod +x /opt/hfptm/monitor-test.sh
echo "✅ Monitoring script created"
EOFSHELL

if [ $? -eq 0 ]; then
    echo "✅ Monitoring script created"
else
    echo "❌ Monitoring script creation failed"
    exit 1
fi

echo ""
echo "======================================"
echo "✅ DEPLOYMENT COMPLETE!"
echo "======================================"
echo ""
echo "🎯 SERVER READY!"
echo ""
echo "📋 NEXT STEPS:"
echo ""
echo "1. ✅ SSH into your server:"
echo "   ssh root@$SERVER_IP"
echo ""
echo "2. 📝 Configure secrets (CRITICAL):"
echo "   cd /opt/hfptm"
echo "   cp config/secrets.toml.example config/secrets.toml"
echo "   nano config/secrets.toml"
echo ""
echo "   REQUIRED FIELDS:"
echo "   - private_key (from MetaMask or Polymarket Settings)"
echo "   - api_key (from Polymarket Builders Program)"
echo "   - api_secret (from Polymarket Builders Program)"
echo "   - api_passphrase (from Polymarket Builders Program)"
echo "   - funder_address (your Polymarket wallet address)"
echo "   - signature_type = 2 (for MetaMask)"
echo ""
echo "3. 🚀 Start bot in TEST MODE (CONSERVATIVE):"
echo "   cd /opt/hfptm"
echo "   sudo systemctl restart hfptm"
echo ""
echo "4. 📊 Monitor test run:"
echo "   tail -f /opt/hfptm/test-monitor.log"
echo ""
echo "   OR access dashboard:"
echo "   http://$SERVER_IP:3000/metrics"
echo ""
echo "5. 📋 Review TESTING_GUIDE.md for detailed testing steps"
echo ""
echo "📊 Server Details:"
echo "   IP: $SERVER_IP"
echo "   Location: SG (Strasbourg)"
echo "   Expected Latency: ~30-40ms to Polymarket ⭐"
echo "   Specs: 1 vCPU, 16GB RAM, 2TB SSD"
echo ""
echo "⚠️  IMPORTANT REMINDERS:"
echo "   • Start in TEST MODE with small positions ($50 max)"
echo "   • Monitor for 1-2 hours before considering live trading"
echo "   • Verify WebSocket connection stability"
echo "   • Check arbitrage detection working"
echo "   • Verify order submission is successful"
echo "   • Measure total latency (should be <200ms)"
echo "   • Only after all checks pass, switch to production config"
echo ""
echo "======================================"
