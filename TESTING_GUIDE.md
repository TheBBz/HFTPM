# 🧪 HFTPM - Full Coverage Integration Testing Guide

> **Complete testing procedures** before going live with real capital  
> For SG (Strasbourg) VPS deployment with **~30-40ms latency to Polymarket**

---

## 📋 Table of Contents

- [Preparation Checklist](#preparation-checklist)
- [Testing Configuration](#testing-configuration)
- [Phase 1: Server Deployment](#phase-1-server-deployment)
- [Phase 2: WebSocket Connection Test](#phase-2-websocket-connection-test)
- [Phase 3: Order Book Reception Test](#phase-3-order-book-reception-test)
- [Phase 4: Arbitrage Detection Test](#phase-4-arbitrage-detection-test)
- [Phase 5: Order Execution Test](#phase-5-order-execution-test)
- [Phase 6: Latency Measurement](#phase-6-latency-measurement)
- [Phase 7: Risk Management Test](#phase-7-risk-management-test)
- [Phase 8: Monitoring & Alerts Test](#phase-8-monitoring-alerts-test)
- [Phase 9: 24-Hour Stability Test](#phase-9-24-hour-stability-test)
- [Success Criteria](#success-criteria)
- [Troubleshooting](#troubleshooting)
- [Going Live](#going-live)

---

## ✅ Preparation Checklist

Complete these **BEFORE** starting testing:

### Account Setup
- [ ] Polymarket account created and funded with USDC ($100+ recommended)
- [ ] MetaMask or Polymarket wallet connected
- [ ] Private key exported and stored securely
- [ ] L2 API credentials obtained (from Builders Program)

### Environment Setup
- [ ] SG (Strasbourg) VPS deployed via `./deploy-ovhcloud.sh`
- [ ] Server IP obtained
- [ ] SSH connection working (`ssh root@SERVER_IP`)
- [ ] Configuration files uploaded (`config/config.test.toml`)

### Knowledge Check
- [ ] Read `README.md` - Full system documentation
- [ ] Read `QUICKSTART.md` - Quick start guide
- [ ] Read `IMPLEMENTATION_SUMMARY.md` - Technical details
- [ ] Understand basic arbitrage concepts (YES+NO < $1.00)

### Tools Ready
- [ ] SSH client installed (or web-based SSH)
- [ ] Terminal ready for log monitoring
- [ ] Browser open for dashboard (`http://SERVER_IP:3000`)
- [ ] Telegram bot created and tested (if configured)

---

## 🧪 Testing Configuration

Your testing config (`config/config.test.toml`) uses:

**Parameters**:
```toml
[trading]
bankroll = 1000              # Your full bankroll
max_arb_size = 50            # CONSERVATIVE for testing
min_edge = 0.020             # 2.0% threshold
min_liquidity = 100          # $100 minimum liquidity
order_type = "FOK"           # Fill-Or-Kill

[risk]
max_exposure_per_market = 50   # Very conservative
max_exposure_per_event = 150    # Conservative
max_concurrent_arbs = 3       # Limit concurrent arbs
daily_loss_limit = 20         # Stop after $20 loss

[markets]
prioritize_categories = ["sports"]  # Focus only on sports
blacklisted_markets = []     # No blacklists
```

**Why Conservative Settings for Testing?**
✅ Reduces risk during testing phase
✅ Easier to interpret results
✅ Smaller positions = less capital at risk if bugs exist
✅ Limits concurrent arbitrage executions
✅ Allows 2-3x more testing cycles with same capital

---

## 🟢 Phase 1: Server Deployment

### Objective
Deploy HFTPM to your SG (Strasbourg) VPS with optimized settings.

### Steps

**1. Run Deployment Script**
```bash
./deploy-ovhcloud.sh
```

**2. Verify Deployment**
```bash
# SSH into server
ssh root@YOUR_SERVER_IP

# Check service status
sudo systemctl status hfptm
# Should show: "active (running)"

# Check logs
sudo journalctl -u hfptm -f

# Should see: "🚀 HFTPM Ultra-Low-Latency Arbitrage Bot Starting"
```

**3. Verify Server Resources**
```bash
# On server
ssh root@YOUR_SERVER_IP

# Check CPU cores
nproc

# Check RAM
free -h

# Check disk
df -h

# Expected: 1 vCPU, 16GB RAM, 2TB SSD
```

### Success Criteria
- [ ] Bot service is running and healthy
- [ ] No errors in startup logs
- [ ] Dashboard accessible at `http://SERVER_IP:3000`
- [ ] System resources match expected specs

### Troubleshooting

**Service won't start:**
```bash
# Check service file exists
sudo cat /etc/systemd/system/hfptm.service

# Check permissions
sudo systemctl daemon-reload

# Manual start
cd /opt/hfptm
./target/release/hfptm --config config/config.test.toml
```

**Can't connect via SSH:**
- Check server IP is correct
- Check firewall rules on VPS
- Check internet connection

**Build fails:**
```bash
# Check Rust is installed
rustc --version

# Check network connection
ping -c 3 google.com

# Retry build
cd /opt/hfptm
cargo build --release
```

---

## 🔵 Phase 2: WebSocket Connection Test

### Objective
Verify WebSocket connection to Polymarket and message reception.

### Test Steps

**1. Monitor Initial Connection**
```bash
# SSH into server
ssh root@YOUR_SERVER_IP

# Watch logs in real-time
sudo journalctl -u hfptm -f

# Look for:
✅ "WebSocket connected to wss://ws-subscriptions-clob.polymarket.com/ws/market"
✅ "Subscribing to 4500+ markets..."
✅ "✅ Configuration loaded successfully"
```

**2. Verify Connection Time**
```bash
# Note the timestamp when you see "WebSocket connected"
# Connection should be <10 seconds from bot start
```

**Expected Behavior**:
```
0:00  → Bot starts, connects to WebSocket
0:02  → Subscribes to markets
0:05  → Receives full order book snapshots
```

**Success Criteria**
- [ ] WebSocket connects within 30 seconds of startup
- [ ] No connection errors in logs
- [ ] Successfully subscribes to markets (4500+)

**Failure Indicators**
- ❌ "WebSocket connection failed"
- ❌ "Reconnecting... " (appears repeatedly)
- ❌ "Connection timeout"

### Troubleshooting

**Connection timeouts or failures:**
```bash
# Test network connectivity from server
ping -c 10 ws-subscriptions-clob.polymarket.com

# Check Polymarket API status
curl https://clob.polymarket.com/ok

# Should return HTTP 200
```

**Manual WebSocket test:**
```bash
# Install wscat
cargo install wscat

# Test connection
wscat -c "wss://ws-subscriptions-clob.polymarket.com/ws/market"
```

**Common Solutions:**
1. Check firewall allows port 443
   ```bash
   sudo ufw status
   
   # If not allowing:
   sudo ufw allow 443/tcp
   sudo ufw enable
   ```

2. Verify server location (SG has best latency)
   ```bash
   # Ping Polymarket WebSocket server
   ping -c 20 ws-subscriptions-clob.polymarket.com
   
   # Should be: 30-50ms
   ```

3. Check system resources
   ```bash
   # CPU and memory usage
   top
   
   # If usage >80%, may cause connection issues
   ```

---

## 📊 Phase 3: Order Book Reception Test

### Objective
Verify order book snapshots and updates are being received correctly.

### Test Steps

**1. Check Initial Snapshots**
```bash
# SSH into server
ssh root@YOUR_SERVER_IP

# Tail logs for order book updates
sudo journalctl -u hfptm -f | grep -i "order book"

# Should see many lines like:
✅ "Updated order book for MARKET_ID asset ASSET_ID"
✅ "📊 Loaded 4500+ markets from Gamma API"
```

**2. Verify Multiple Markets**
```bash
# Check logs for different market IDs
sudo journalctl -u hfptm -f | grep "Updated order book" | head -20

# Should see order books for various markets
```

**3. Monitor Message Rate**
```bash
# Count messages per minute
grep "WS message" /opt/hfptm/logs/hfptm.log | wc -l

# Expected: 100-500+ messages/minute
# If <100: Check WebSocket connection
```

### Success Criteria
- [ ] Order book updates received for 10+ different markets within first minute
- [ ] Message rate >100 msg/s
- [ ] No order book parsing errors in logs
- [ ] Timestamps show recent data (not stale)

**Failure Indicators**
- ❌ "Failed to parse WebSocket message"
- ❌ "Order book is stale" (timestamp >5 seconds old)
- ❌ Very low message rate (<50 msg/s)

### Troubleshooting

**No order book updates:**
```bash
# Check if markets were loaded
grep "Loaded" /opt/hfptm/logs/hfptm.log

# If empty:
# - Check Gamma API connectivity
# - Verify market filtering in config
```

**Parsing errors:**
```bash
# Look for JSON parse errors
grep "Failed to parse" /opt/hfptm/logs/hfptm.log

# Common causes:
# - Invalid WebSocket message format
# - Network corruption
# - Polymarket API changes
```

---

## 🎯 Phase 4: Arbitrage Detection Test

### Objective
Verify arbitrage detection logic is working correctly.

### Test Steps

**1. Run for 15-30 minutes during active period**
```bash
# SSH into server
ssh root@YOUR_SERVER_IP

# Monitor detection logs
sudo journalctl -u hfptm -f | grep "Arbitrage detected"

# Wait for detections
```

**2. Check Detection Rate**
```bash
# Count detections in last hour
grep "Arbitrage detected" /opt/hfptm/logs/hfptm.log | tail -60 | wc -l

# Expected: 5-15 detections/hour during testing
# Testing config has conservative settings, so may be lower
```

**3. Verify Detection Types**
```bash
# Check if both binary and multi-outcome are detected
grep "Arbitrage detected" /opt/hfptm/logs/hfptm.log | tail -20

# Should see both:
# - Binary: "arb_type": "Binary"
# - Multi-outcome: "arb_type": "MultiOutcome"
```

**4. Check Edge Calculation**
```bash
# Verify edge amounts are reasonable
grep -E "edge: ([0-9]\.[0-9]*)" /opt/hfptm/logs/hfptm.log | tail -10

# Expected edges:
# - Testing config: 2.0% min edge, so should see 2.0-5.0%
# - Good opportunities: 2.5% - 5.0% edge
# - Poor opportunities: <2.0% (rare but may still trigger)
```

### Success Criteria
- [ ] 5-15 arbitrage opportunities detected in 30-minute window
- [ ] Edges fall within expected range (2.0% - 5.0%)
- [ ] Detection timestamps show recent data
- [ ] No detection errors in logs
- [ ] Detection latency <50ms (look for "detection latency" in logs)

**Failure Indicators**
- ❌ Zero detections after 30 minutes of active markets
- ❌ All edges outside expected range (<2.0% or >10%)
- ❌ Detection errors repeated frequently
- ❌ Detection latency >100ms

### Troubleshooting

**No or very few detections:**
```bash
# 1. Check WebSocket connection (see Phase 2)
# 2. Verify markets are subscribed
grep "Subscribing" /opt/hfptm/logs/hfptm.log | tail -5

# 3. Check if market filtering is too restrictive
nano config/config.test.toml
# Try removing categories from prioritize_categories
# Try lowering min_edge to 0.015

# 4. Monitor during high-liquidity periods
# Sports during live events
# Crypto during high volatility
```

**Inaccurate edge calculations:**
```bash
# Check logs for calculation
grep "total_edge" /opt/hfptm/logs/hfptm.log | tail -10

# If edges are consistently outside expected range:
# - Check order book data integrity
# - Verify pricing calculation in arb_engine/mod.rs
# - Check min_edge configuration
```

---

## 🚀 Phase 5: Order Execution Test

### Objective
Verify order creation, signing, and submission work correctly.

### Test Steps

**1. Wait for First Detection**
```bash
# Monitor logs for first arbitrage detection
sudo journalctl -u hfptm -f | grep -m "Arbitrage #1 detected"

# Note the market ID and expected profit
```

**2. Monitor Order Creation**
```bash
# Check for order creation logs
grep "Created.*signed orders" /opt/hfptm/logs/hfptm.log | tail -10

# Should see:
✅ "📦 Created 2 signed orders for MARKET_ID"
```

**3. Monitor Order Submission**
```bash
# Check for submission results
grep "submit.*orders" /opt/hfptm/logs/hfptm.log | tail -10

# Should see:
✅ "✅ Execution results: 2/2 orders filled"
```

**4. Verify Success Rate**
```bash
# Count successful vs failed executions
grep "success: true" /opt/hfptm/logs/hfptm.log | wc -l
grep "success: false" /opt/hfptm/logs/hfptm.log | wc -l

# Expected with test config:
# - 70-90% success rate (conservative settings)
# - Should see 3-4 successful executions in first hour
```

### Success Criteria
- [ ] Orders created successfully (no signing errors)
- [ ] Orders submitted to Polymarket API (no API failures)
- [ ] 50%+ of orders filled (with FOK order type)
- [ ] Execution time <200ms (see logs)
- [ ] No critical errors in submission

**Failure Indicators**
- ❌ "Failed to create order" repeated
- ❌ "Order submission failed" with API errors
- ❌ "success: false" >70% of submissions
- ❌ Execution time >500ms

### Troubleshooting

**Signing failures:**
```bash
# Check for signing errors
grep -E "Failed to sign|SIGNATURE_ERROR" /opt/hfptm/logs/hfptm.log | tail -10

# Common causes:
# - Invalid private key format
# - API credentials not configured
# - Signature type mismatch (should be 2 for MetaMask)
```

**Submission failures:**
```bash
# Check API errors
grep -E "FAILED|ERROR.*submission|HTTP 40[0-9]" /opt/hfptm/logs/hfptm.log | tail -10

# Common causes:
# - Insufficient balance
# - Invalid order parameters
# - Rate limit exceeded
# - API service unavailable
```

**Low fill rate:**
```bash
# Check partial fills
grep -E "filled.*[0-9]" /opt/hfptm/logs/hfptm.log | tail -10

# If seeing many partial fills:
# - Increase slippage tolerance in config
# - Reduce position size
# - Consider using GTC instead of FOK
```

---

## ⏱ Phase 6: Latency Measurement

### Objective
Measure end-to-end latency from detection to order submission.

### Test Steps

**1. Find Latency Measurements**
```bash
# Search for execution times in logs
grep "Arbitrage executed.*in.*ms" /opt/hfptm/logs/hfptm.log | tail -10

# Should see lines like:
# "✅ Arbitrage executed: MARKET_ID in 125ms"
```

**2. Calculate Statistics**
```bash
# Extract latency values
grep "Arbitrage executed" /opt/hfptm/logs/hfptm.log | grep -oP "in [0-9]*ms" | sed 's/.*in \([0-9]*\) ms/\1/'

# Get average
# Average of all values

# Get p50/p99
# Sort values, pick 50th and 99th percentile
```

### Success Criteria
- [ ] Average latency <200ms (target for test mode)
- [ ] P50 latency <150ms (excellent)
- [ ] P99 latency <300ms (acceptable)
- [ ] Latency variance is low (consistent performance)

**Failure Indicators**
- ❌ Average latency >200ms
- ❌ Average latency >300ms
- ❌ High latency variance (>100ms swing)
- ❌ Latency spikes >500ms

### Troubleshooting

**High latency causes:**
```bash
# 1. Check network latency from server
ping -c 20 ws-subscriptions-clob.polymarket.com

# 2. Check CPU usage
htop

# 3. Check WebSocket message rate
grep "WS message" /opt/hfptm/logs/hfptm.log | wc -l

# 4. Check if order book manager is bottleneck
grep "book_snapshot" /opt/hfptm/logs/hfptm.log | tail -10
```

**Solutions:**
1. If network latency >50ms:
   - Consider different data center (still OVHcloud is good)
   - Use premium RPC endpoint (QuickNode Pro)
   - Contact OVHcloud support

2. If CPU usage >80%:
   - Check for background processes
   - Reduce max_order_books in config
   - Enable CPU pinning

3. If order book processing slow:
   - Check for memory leaks
   - Verify lock-free structures are working
   - Reduce market count

---

## 🛡 Phase 7: Risk Management Test

### Objective
Verify risk limits are working and protecting capital.

### Test Steps

**1. Check Risk Manager Activity**
```bash
# Monitor risk manager logs
sudo journalctl -u hfptm -f | grep -i "risk"

# Should see:
# - "Risk manager rejected arbitrage"
# - "Recorded arbitrage execution"
# - "Active arbitrages: X"
```

**2. Test Exposure Limits**
```bash
# Create multiple small arbitrage opportunities
# Let bot detect and attempt execution

# Wait for rejections
# Should see:
# "⚠️  Market exposure limit reached"
# - "⚠️  Event exposure limit reached"
```

**3. Test Daily Loss Limit**
```bash
# Monitor cumulative PnL
grep "daily_pnl" /opt/hfptm/logs/hfptm.log | tail -10

# If approaching limit ($15-18 loss):
# Should see warnings

# If reached limit ($20 loss):
# Should see:
# "⚠️  Daily loss limit reached"
# - Bot should stop trading
```

**4. Test Inventory Tracking**
```bash
# Check position logs
grep "Recorded position" /opt/hfptm/logs/hfptm.log | tail -10

# Should see positions being tracked
# Verify delta-neutral balance
```

### Success Criteria
- [ ] Risk limits correctly enforced (no unauthorized large trades)
- [ ] Daily loss limit prevents excessive losses
- [ ] Inventory tracking shows all positions
- [ ] Concurrent arbitrage limit respected (≤3 active)

**Failure Indicators**
- ❌ Trades exceeding exposure limits
- ❌ Daily loss limit reached multiple times
- ❌ Position tracking missing or incorrect
- ❌ >3 concurrent arbitrages simultaneously

### Troubleshooting

**Risk limits not enforced:**
```bash
# Verify config settings
cat config/config.test.toml | grep -A 20 risk

# Check if risk manager is receiving config
# Logs should show risk checks before execution
```

**Inventory drift too large:**
```bash
# Check inventory_drift_threshold setting
# Should be 0.05 (5%)
# If delta exceeds threshold, rebalancing occurs
```

---

## 📈 Phase 8: Monitoring & Alerts Test

### Objective
Verify dashboard, metrics, and alerting systems are working.

### Test Steps

**1. Access Dashboard**
```bash
# Open in browser
http://SERVER_IP:3000/metrics

# Should see real-time metrics
```

**2. Verify Metrics Endpoint**
```bash
# Test endpoint
curl http://SERVER_IP:3000/metrics | jq

# Should return JSON with:
# - arb_detections
# - arb_executions
# - avg_latency_ms
# - websocket_connected
# - active_positions
```

**3. Check Trade History**
```bash
# Access trades endpoint
http://SERVER_IP:3000/trades?limit=10

# Should show recent trades
```

**4. Test Alerts**
```bash
# Look for alert logs
grep -i "alert\|Alert" /opt/hfptm/logs/hfptm.log | tail -10

# Should see:
# - Trade executed alerts
# - Error alerts
# - Latency spike alerts
```

### Success Criteria
- [ ] Dashboard accessible and updating in real-time
- [ ] Metrics endpoint returns valid JSON
- [ ] Trade history shows executed trades
- [ ] Alerts are being generated correctly
- [ ] Telegram alerts received (if configured)

**Failure Indicators**
- ❌ Dashboard returns errors or is down
- ❌ Metrics endpoint returns HTTP errors
- [ ] Trade history not updating
- ❌ Alerts generated but not sent via Telegram

### Troubleshooting

**Dashboard inaccessible:**
```bash
# Check service is running
sudo systemctl status hfptm

# Check dashboard port is listening
sudo netstat -tulpn | grep :3000

# Should show LISTEN on 0.0.0.0:3000
```

**Metrics not updating:**
```bash
# Check bot is still running
ps aux | grep hfptm

# Check for error logs
tail -20 /opt/hfptm/logs/hfptm.log
```

**Telegram alerts not working:**
```bash
# Verify configuration
cat config/config.test.toml | grep -A 10 alerts

# Test bot token manually
curl -X POST "https://api.telegram.org/bot<token>/sendMessage?chat_id=<chat_id>&text=test" | jq

# Check logs for errors
grep "Failed to send Telegram alert" /opt/hfptm/logs/hfptm.log | tail -10
```

---

## ⏱ Phase 9: 24-Hour Stability Test

### Objective
Run bot continuously for 24 hours to verify stability.

### Test Steps

**1. Start Test Run**
```bash
# Ensure bot is running
sudo systemctl start hfptm

# Note start time
date
```

**2. Monitor Resources**
```bash
# Open SSH session for monitoring
ssh root@YOUR_SERVER_IP

# In separate terminal, run:
htop

# Monitor:
# - CPU usage (should be 40-60% for idle, 80-90% under load)
# - RAM usage (should be <80%)
# - Network stability
```

**3. Monitor Logs**
```bash
# In monitoring session:
tail -f /opt/hfptm/logs/hfptm.log

# Watch for:
# - WebSocket disconnects
# - Error spikes
# - Memory leaks
# - CPU spikes
```

**4. Periodic Health Checks**
```bash
# Every 2 hours:
curl http://SERVER_IP:3000/health

# Should return: "status": "healthy"
```

**5. Collect Metrics**
```bash
# At end of 24 hours:
curl http://SERVER_IP:3000/metrics | jq

# Record final values
```

### Success Criteria
- [ ] Uptime >99.5% (bot running continuously)
- [ ] Zero crashes or uncontrolled shutdowns
- [ ] No memory leaks (stable RAM usage)
- [ ] <5 WebSocket disconnects in 24 hours
- [ ] Error rate <0.1% of total messages
- [ ] Avg latency remains stable

**Failure Indicators**
- ❌ Bot crashes or restarts frequently (>3 times in 24 hours)
- ❌ RAM usage increases over time (memory leak)
- ❌ Frequent WebSocket disconnects (>10 in 24 hours)
- ❌ Error rate >1% of total messages
- ❌ Latency degrades over time

### Troubleshooting

**Frequent crashes:**
```bash
# Check logs for panic messages
grep -i "panic\|unwrap\|thread" /opt/hfptm/logs/hfptm.log | tail -20

# Look for:
# - "thread 'main' panicked"
# - "Arithmetic error"
# - "index out of bounds"
```

**Memory leaks:**
```bash
# Monitor memory usage trend
free -h

# If consistently increasing:
# - Restart bot
# - Investigate order book manager
# - Reduce max_order_books
```

**WebSocket disconnects:**
```bash
# Count disconnects
grep -i "reconnecting\|WebSocket closed" /opt/hfptm/logs/hfptm.log | wc -l

# If >5 in 24 hours:
# - Check network quality
# - Check Polymarket API status
# - Consider increasing reconnect delay
```

---

## ✅ Success Criteria

**ALL PHASES MUST PASS BEFORE GOING LIVE:**

### Core Requirements
- [ ] WebSocket connects successfully within 30 seconds
- [ ] Receives order book updates for 10+ markets
- [ ] Arbitrage detection working (5-15+ per hour)
- [ ] Order execution working (50%+ success rate)
- [ ] End-to-end latency <200ms average
- [ ] Risk limits enforcing correctly
- [ ] Dashboard accessible and updating
- [ ] Bot runs stable for 24+ hours

### Performance Metrics (Test Mode Targets)
- [ ] **Detection Latency**: <50ms
- [ ] **Execution Latency**: <100ms
- [ ] **Total Latency**: <200ms
- [ ] **Throughput**: 5-15 arbs/hour
- [ ] **Capture Rate**: 50-70%
- [ ] **Uptime**: >99.5% over 24 hours

### Configuration (Before Going Live)
```toml
[trading]
bankroll = 1000              # Your bankroll
max_arb_size = 100           # Ready for live trading
min_edge = 0.025             # 2.5% minimum profit
min_liquidity = 100          # $100 minimum liquidity

[risk]
max_exposure_per_market = 200  # Increased for live
max_exposure_per_event = 500    # Increased for live
max_concurrent_arbs = 10     # Increased for live
daily_loss_limit = 50         # Reasonable daily limit
```

---

## 🛑 Going Live

### Prerequisites (MUST COMPLETE)
- [ ] All 9 testing phases passed successfully
- [ ] Server has been stable for 24+ hours
- [ ] You're comfortable with test results
- [ ] Risk limits adjusted appropriately
- [ ] Telegram alerts working (optional but recommended)
- [ ] You understand the risks and are willing to start small

### Preparation Steps

**1. Update Configuration for Live Trading**
```bash
# SSH into server
ssh root@YOUR_SERVER_IP

# Edit config
nano config/config.toml

# Update to live settings:
[trading]
bankroll = 1000              # Your bankroll
max_arb_size = 200           # PRODUCTION size (10x test)
min_edge = 0.025             # Keep 2.5% for volume

[risk]
max_exposure_per_market = 200  # Increased
max_exposure_per_event = 500    # Increased
max_concurrent_arbs = 10      # Increased
daily_loss_limit = 50         # Increased
```

**2. Restart Bot with Live Configuration**
```bash
# Apply changes
sudo systemctl daemon-reload

# Restart bot
sudo systemctl restart hfptm

# Verify new settings
sudo journalctl -u hfptm -f | grep -E "max_arb_size.*200|bankroll.*1000"

# Should see:
# "📊 Bankroll: $1000 USDC"
```

**3. Monitor First Live Trades**
```bash
# Watch logs
sudo journalctl -u hfptm -f

# Look for:
# - "Arbitrage executed" with $200 positions
# - Execution times <200ms
# - Success rates >70%

# Wait for 2-5 successful trades
```

### Scaling Plan

**Week 1: Foundation**
- Target: 20-40 arbs/day
- Position size: $100-200
- Focus: Learn system behavior, refine detection

**Week 2: Optimization**
- Target: 50-100 arbs/day
- Position size: $200-500 (based on edge)
- Focus: Tune parameters, improve capture rate

**Week 3: Scale**
- Target: 100-200 arbs/day
- Position size: $500-1000
- Focus: Full capacity with proven settings

**Week 4: Maximize**
- Target: 200+ arbs/day
- Position size: $1000-2000
- Focus: Maximum compounding

---

## 🔧 Troubleshooting

### Common Issues During Testing

**Bot won't start:**
```bash
# Check logs
sudo journalctl -u hfptm -n 50

# Common errors:
# - "Private key must be set"
# - "API key must be set"
# - "Failed to parse WebSocket message"

# Solutions:
# - Verify config/secrets.toml is configured
# - Check API credentials
# - Verify network connectivity
```

**No arbitrage detected (Phase 4):**
- **Verify WebSocket is receiving order book updates**
- **Check market filtering in config**
- **Consider lowering min_edge to 0.015**
- **Monitor during active periods (sports games, crypto volatility)**
- **Verify markets are in active state**

**Orders failing (Phase 5):**
- **Check for API errors in logs**
- **Verify sufficient USDC balance**
- **Check funder address matches Polymarket wallet**
- **Try smaller position sizes**
- **Check Polymarket status: `curl https://clob.polymarket.com/ok`

**High latency (Phase 6):**
- **Run: `ping -c 20 ws-subscriptions-clob.polymarket.com`**
- **Check CPU governor: `cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor`**
- **Should be: "performance"`
- **Verify system is not overloaded**
- **Consider different data center if consistently >200ms**

**Risk limits not working (Phase 7):**
- **Verify risk config settings match your expectations**
- **Check if risk manager is receiving updated config**
- **Monitor for rejections in logs**
- **Reduce exposure limits if too conservative**

**Dashboard inaccessible (Phase 8):**
- **Verify bot is running: `sudo systemctl status hfptm`**
- **Check network connectivity**
- **Test port 3000: `curl http://SERVER_IP:3000/health`**
- **Check service is listening: `sudo netstat -tulpn | grep :3000`**

---

## 📞 Quick Reference

### Essential Commands

```bash
# SSH into server
ssh root@SERVER_IP

# Monitor logs (real-time)
sudo journalctl -u hfptm -f

# Check bot status
sudo systemctl status hfptm

# Restart bot
sudo systemctl restart hfptm

# Stop bot
sudo systemctl stop hfptm

# View recent logs
sudo journalctl -u hfptm -n 100

# Check resources
free -h
htop

# Edit config
nano config/config.toml
```

### Monitoring URLs

```bash
# Metrics dashboard
http://SERVER_IP:3000/metrics

# Trade history
http://SERVER_IP:3000/trades?limit=50

# Alerts history
http://SERVER_IP:3000/alerts?limit=50

# Health check
http://SERVER_IP:3000/health
```

---

## 📊 Performance Expectations

### During Testing (conservative config)

| Metric | Expected | Acceptable Range |
|--------|----------|-----------------|
| **Detections/hour** | 5-15 | 3-10 |
| **Executions/hour** | 3-8 | 2-5 |
| **Success rate** | 50-70% | 40-60% |
| **Avg latency** | 100-190ms | <250ms |
| **Uptime** | >99% | >98% |

### Going Live (optimized config)

| Metric | Expected | Acceptable Range |
|--------|----------|-----------------|
| **Detections/hour** | 10-25 | 5-15 |
| **Executions/hour** | 8-15 | 5-12 |
| **Success rate** | 70-80% | 60-75% |
| **Avg latency** | 120-180ms | <200ms |
| **Uptime** | >99.5% | >99% |
| **Daily profit** | $100-300 | $50-200 (first week) |
| **Monthly profit** | $3,000-6,000 | $12,000-24,000 (month 1, compounding) |

---

## 🎯 Success Criteria Summary

### Before Going Live - ALL MUST BE ✅:

**Infrastructure**:
- [ ] SG (Strasbourg) VPS deployed and stable
- [ ] Bot service running via systemd
- [ ] System resources: 1 vCPU, 16GB RAM, 2TB SSD
- [ ] Network: ~30-40ms to Polymarket ⭐

**Functionality**:
- [ ] WebSocket connects and stays connected
- [ ] Order books received for 10+ markets
- [ ] Arbitrage detection working
- [ ] Order execution successful (50%+ rate)
- [ ] End-to-end latency <200ms average
- [ ] Risk limits enforcing correctly
- [ ] Dashboard accessible and updating

**Testing**:
- [ ] 24+ hours stability test passed
- [ ] All 9 testing phases completed successfully
- [ ] Performance metrics meet targets
- [ ] No critical errors detected
- [ ] You're comfortable with test results

**Configuration**:
- [ ] Live config prepared with appropriate risk limits
- [ ] Scaling plan in place
- [ ] You understand operation and risks
- [ ] Telegram alerts configured (optional but recommended)

---

## ⚖️ Final Reminders

⚠️ **Start Small**: Begin with $100 position sizes, scale gradually
⚠️ **Monitor Closely**: Watch first few trades for 24-48 hours
⚠️ **Have Exit Strategy**: Know when to stop (loss limits, errors, etc.)
⚠️ **Keep Learning**: Adjust parameters based on live performance
⚠️ **Risk Management**: Never exceed your comfort zone
⚠️ **Compound Gradually**: Reinvest profits for exponential growth

---

## 📞 Support Resources

**Documentation**:
- README.md - Full system documentation
- IMPLEMENTATION_SUMMARY.md - Technical details
- This guide (TESTING_GUIDE.md)

**Polymarket**:
- Discord: https://discord.gg/polymarket
- Documentation: https://docs.polymarket.com/developers/CLOB/introduction
- WebSocket: https://docs.polymarket.com/developers/CLOB/websocket/market-channel

**Troubleshooting**:
- Check logs: `sudo journalctl -u hfptm -f -n 50`
- GitHub Issues: Report bugs and feature requests
- Community support: Join Polymarket Discord

---

<div align="center">

**✅ YOU'RE READY FOR LIVE TRADING!**

<div align="center">

**Complete all 9 testing phases before going live. Start small, monitor closely, and scale gradually.** ⭐

</div>
