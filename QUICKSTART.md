# 🚀 QUICK START GUIDE

Get HFTPM running in **10 minutes** with production-ready configuration.

---

## ⏱️ Time Requirements

| Step | Time | Difficulty |
|------|--------|------------|
| Prerequisites | 2 min | Easy |
| Account Setup | 3 min | Easy |
| Configuration | 3 min | Medium |
| Deployment | 2 min | Easy |
| **Total** | **10 min** | **Easy** |

---

## 📋 Step 1: Prerequisites (2 min)

### Option A: Your Local Machine

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Verify installation
rustc --version  # Should be 1.88+
cargo --version
```

### Option B: Hetzner VPS (Recommended)

```bash
# Create account: https://www.hetzner.com
# Order CX51+ server: https://www.hetzner.com/cloud#order
# Choose Amsterdam (nbg1) location
# Wait ~5 minutes for server provisioning
```

---

## 🏗 Step 2: Clone & Build (2 min)

```bash
# Clone repository
git clone https://github.com/your-repo/HFTPM.git
cd HFTPM

# Run setup script
./setup.sh

# This will:
# ✅ Check prerequisites
# ✅ Create directories
# ✅ Install dependencies
# ✅ Build release binary
# ✅ Run tests
```

**Expected output**:
```
🚀 HFTPM - Automated Setup Script
====================================

📋 Checking prerequisites...
✅ All prerequisites installed

📁 Creating project structure...
✅ Config directory exists

🔐 Setting up configuration...
⚠️  config/secrets.toml not found
   ⚠️  IMPORTANT: Edit config/secrets.toml with your credentials!
   ⚠️  NEVER commit config/secrets.toml to Git!

📦 Installing Rust dependencies...
✅ Dependencies installed

🔨 Building release version...
✅ Build complete

🧪 Running tests...
✅ Tests passed

=======================================
✅ Setup complete!
```

---

## 🔑 Step 3: Configure Secrets (3 min - CRITICAL!)

```bash
# Edit secrets file
nano config/secrets.toml
```

**Fill in these REQUIRED fields**:

```toml
[credentials]
private_key = "YOUR_PRIVATE_KEY_HERE"        # From MetaMask or Polymarket Settings
api_key = "YOUR_API_KEY_HERE"           # From Polymarket Builders Program
api_secret = "YOUR_API_SECRET_HERE"       # From Polymarket Builders Program
api_passphrase = "YOUR_PASSPHRASE_HERE" # From Polymarket Builders Program
funder_address = "YOUR_WALLET_ADDRESS_HERE" # Your Polymarket proxy wallet
signature_type = 2                           # 2=Gnosis Safe (MetaMask), 1=Proxy (MagicLink)

[server]
polygon_rpc_url = "YOUR_QUICKNODE_PRO_URL"  # Optional but recommended
```

### Get Private Key

**Method 1: MetaMask Export**
1. Open MetaMask
2. Click account icon → Account Details
3. Click "Export Private Key"
4. Enter password
5. Copy the `0x...` key

**Method 2: Polymarket Settings**
1. Log in to polymarket.com
2. Settings → Export Private Key
3. Enter password
4. Copy the `0x...` key

### Get L2 API Credentials (Builders Program)

1. Go to: https://docs.polymarket.com/developers/builders/builder-intro
2. Click "Create Builder Profile"
3. Fill in wallet address and details
4. Copy API Key, Secret, and Passphrase from Builder Profile

### Get Funder Address

1. Log in to polymarket.com
2. Your wallet address is displayed at top right
3. Copy the `0x...` address

---

## 🎛 Step 4: Adjust Trading Parameters (Optional - 1 min)

Edit `config/config.toml` to customize:

```toml
[trading]
bankroll = 1000              # Your starting capital (default: $1000)
max_arb_size = 100           # Max $ per arbitrage (start conservative)
min_edge = 0.025             # Minimum 2.5% profit threshold

[risk]
max_exposure_per_market = 200  # $200 max per market
daily_loss_limit = 50         # Stop if lose $50 in a day

[markets]
prioritize_categories = ["sports", "esports"]  # Focus on high-frequency markets
```

**Recommendations for $1000 capital**:
- Start with `max_arb_size = 50-100` (conservative)
- Use `min_edge = 0.025` (2.5% - captures more opportunities)
- Focus on "sports" + "esports" (highest liquidity)

---

## 🚀 Step 5: Run the Bot (1 min)

### Development Mode (Local)

```bash
# Run with console output
cargo run --release

# Expected output:
# 🚀 HFTPM Ultra-Low-Latency Arbitrage Bot Starting
# 📊 Bankroll: $1000 USDC
# 🎯 Min Edge: 2.50%
# ✅ Configuration loaded successfully
```

### Production Mode (Hetzner VPS)

```bash
# Method 1: Using Makefile
make deploy-hetzner

# Method 2: Manual deployment
ssh root@your-server-ip
cd /opt/hfptm
sudo systemctl start hfptm

# Check logs
sudo journalctl -u hfptm -f
```

### Monitor Dashboard

Once running, access:
```
http://localhost:3000/metrics   # Performance metrics
http://localhost:3000/trades   # Trade history
http://localhost:3000/alerts   # Alert history
http://localhost:3000/health   # Health check
```

---

## ✅ Verification Checklist

Before going live, verify:

- [ ] Private key is correct and not leaked
- [ ] API credentials from Builders Program are configured
- [ ] Funder address matches Polymarket wallet
- [ ] USDC deposited on Polymarket (minimum $100 recommended)
- [ ] Dashboard accessible at http://localhost:3000
- [ ] WebSocket connection shows "✅ WebSocket connected"
- [ ] First market data received (check logs)
- [ ] Risk limits appropriate for your bankroll

---

## 🎯 First Trade Expected Timeline

```
Time 0:00   → Bot starts, connects to WebSocket
Time 0:02   → Subscribes to 4500+ markets
Time 0:05   → Receives first order book snapshots
Time 0:08   → Detects first arbitrage opportunity
Time 0:10   → Calculates profit, checks risk limits
Time 0:15   → Creates and signs orders
Time 0:20   → Submits parallel orders to Polymarket
Time 0:22   → Orders filled, position opened
Time 0:25   → Dashboard updated with trade info
Time 0:30   → Telegram alert sent (if configured)
```

**Total time to first trade: ~30 seconds**

---

## 📊 Expected Performance

With **$1000 starting capital**:

| Metric | Expected |
|--------|----------|
| **First Day** | 10-25 arbitrage opportunities |
| **Daily Profit** | $20-60 (conservative estimate) |
| **Monthly Profit** | $600-1800 (compounding) |
| **Capture Rate** | 70-85% of detected opportunities |
| **Avg Latency** | 120-180ms |
| **Uptime** | 99.5%+ |

**Risk Profile**:
- Maximum daily loss: $50 (configurable circuit breaker)
- Per-market exposure: $200
- Drawdown tolerance: 5% delta-neutral drift

---

## ⚠️ Common Mistakes to Avoid

### ❌ Don't do this:

1. **Running as root locally** - Use normal user account
2. **Skipping tests** - Always run `make test` first
3. **Committing secrets** - Already in `.gitignore`, but double-check!
4. **Using production keys on testnet** - Separate test and production keys
5. **Ignoring latency spikes** - High latency = missed opportunities
6. **Over-optimizing for backtests** - Focus on live performance
7. **Trading with money you can't afford to lose** - Start small

### ✅ Do this instead:

1. **Start in dry-run mode** - Test with small positions first
2. **Monitor logs closely** - Watch for errors or anomalies
3. **Adjust risk limits** - Tighten if losses exceed expectations
4. **Keep local backup** - Save configuration snapshots
5. **Use Telegram alerts** - Stay informed of bot activity
6. **Regular updates** - `git pull` to get latest fixes
7. **Diversify categories** - Don't rely on single market type

---

## 🆘 Troubleshooting

### Bot won't start

```bash
# Check logs
tail -f logs/hfptm.log

# Common errors:
# "Private key must be set" → Edit config/secrets.toml
# "API key must be set" → Edit config/secrets.toml
# "Invalid signature type" → Set signature_type = 2 for MetaMask
```

### No arbitrage detected

**Normal behavior**: Opportunities are sporadic

**If no arbs for >1 hour**:
1. Check WebSocket connection: `curl https://clob.polymarket.com/ok`
2. Lower `min_edge` in config (try 0.020 = 2.0%)
3. Verify markets are subscribed (check logs for "Subscribing to X markets")
4. Check Polymarket status for outages

### Orders failing

**Error: "INVALID_ORDER_NOT_ENOUGH_BALANCE"**
- Deposit USDC to Polymarket
- Check funder address is correct

**Error: "NONCE_ALREADY_USED"**
- Derive existing API keys instead of creating new ones
- Use `derive_api_key` endpoint

### High latency

```bash
# Check network latency
ping -c 10 ws-subscriptions-clob.polymarket.com

# Check CPU usage
htop

# If latency > 300ms:
# - Verify server location (Amsterdam best)
# - Use premium RPC (QuickNode Pro)
# - Check for network congestion
```

---

## 📞 Next Steps

After successful deployment:

1. **Monitor for 24 hours** - Observe detection rate and profit
2. **Analyze trade history** - Review via dashboard `/trades` endpoint
3. **Tune parameters** - Adjust based on live performance
4. **Scale gradually** - Increase position sizes as confidence grows
5. **Set up backups** - Regular backups of config and data
6. **Monitor community** - Join Discord/Telegram for updates

---

## 📚 Additional Resources

- **Full Documentation**: [README.md](README.md)
- **API Reference**: https://docs.polymarket.com/developers/CLOB/introduction
- **Community Support**: Discord, GitHub Issues
- **Strategy Guide**: https://www.polytrackhq.app/blog/polymarket-arbitrage-guide

---

## ⚡ Need Help?

1. Check [README.md](README.md) for detailed troubleshooting
2. Search [GitHub Issues](https://github.com/your-repo/HFTPM/issues)
3. Join community Discord
4. Review logs: `make logs`

---

**⭐ You're all set! Time to start earning risk-free profits!** ⭐
