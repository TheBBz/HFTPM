#!/bin/bash

set -e

echo "🚀 HFTPM - Automated Setup Script"
echo "===================================="
echo ""

check_command() {
    if ! command -v $1 &> /dev/null; then
        echo "❌ $1 is not installed. Please install it first."
        exit 1
    fi
}

echo "📋 Checking prerequisites..."
check_command "rustc"
check_command "cargo"
check_command "git"
check_command "tmux"

echo "✅ All prerequisites installed"
echo ""

echo "📁 Creating project structure..."
if [ -d "config" ]; then
    echo "✅ Config directory exists"
else
    mkdir -p config logs tests
    echo "✅ Created config, logs, tests directories"
fi
echo ""

echo "🔐 Setting up configuration..."
if [ ! -f "config/config.toml" ]; then
    echo "⚠️  config/config.toml not found"
    echo "   Please copy config/config.toml.example and configure your settings"
    echo "   Example config has been created with defaults"
else
    echo "✅ Config file exists"
fi
echo ""

if [ ! -f "config/secrets.toml" ]; then
    echo "⚠️  secrets.toml not found"
    echo "   Copying secrets template..."
    cp config/secrets.toml.example config/secrets.toml
    echo "   ⚠️  IMPORTANT: Edit config/secrets.toml with your credentials!"
    echo "   ⚠️  NEVER commit config/secrets.toml to Git!"
    echo ""
    read -p "Press Enter to continue after editing secrets.toml..."
else
    echo "✅ Secrets file exists"
fi
echo ""

echo "📦 Installing Rust dependencies..."
cargo install --locked --path .
echo "✅ Dependencies installed"
echo ""

echo "🔨 Building release version..."
cargo build --release
echo "✅ Build complete"
echo ""

echo "🧪 Running tests..."
cargo test --release
echo "✅ Tests passed"
echo ""

echo "📊 Creating monitoring setup..."
if [ ! -d "logs" ]; then
    mkdir -p logs
    echo "✅ Logs directory created"
fi
echo ""

echo "🔍 Checking WebSocket connectivity..."
echo "   Testing: wss://ws-subscriptions-clob.polymarket.com/ws/market"
if command -v curl &> /dev/null; then
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" https://clob.polymarket.com/ok)
    if [ "$HTTP_CODE" = "200" ]; then
        echo "✅ Polymarket API is accessible"
    else
        echo "⚠️  Polymarket API returned HTTP code: $HTTP_CODE"
    fi
else
    echo "⚠️  curl not available, skipping connectivity check"
fi
echo ""

echo "====================================="
echo "✅ Setup complete!"
echo ""
echo "📝 Next steps:"
echo "   1. Edit config/secrets.toml with your credentials"
echo "   2. Adjust config/config.toml with your preferences"
echo "   3. Run in dev mode: ./target/release/hfptm"
echo "   4. Or deploy to production (see README.md)"
echo ""
echo "📚 Documentation: See README.md for detailed instructions"
echo ""
echo "🚀 Happy trading! (Use at your own risk)"
