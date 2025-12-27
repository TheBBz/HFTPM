# 📋 Quick Reference - HFTPM Test Phase

> **Essential commands and configurations** for testing your SG (Strasbourg) VPS

---

## 🖥 Server Connection

```bash
# SSH into your SG (Strasbourg) VPS
ssh root@YOUR_SERVER_IP

# Check service status
sudo systemctl status hfptm

# View live logs
sudo journalctl -u hfptm -f -n 100

# Restart bot if needed
sudo systemctl restart hfptm

# Stop bot for maintenance
sudo systemctl stop hfptm
```

---

## 📊 Dashboard Access

**Metrics Dashboard**:
```
http://YOUR_SERVER_IP:3000/metrics
```

**Trade History**:
```
http://YOUR_SERVER_IP:3000/trades?limit=50
```

**Alerts**:
```
http://YOUR_SERVER_IP:3000/alerts?limit=50
```

---

## 🔧 Configuration Files

### Production Config
```bash
# SSH into server
ssh root@YOUR_SERVER_IP
cd /opt/hfptm

# Edit production config
nano config/config.toml
```

### Test Config
```bash
# SSH into server
ssh root@YOUR_SERVER_IP
cd /opt/hfptm

# Edit test config (ALREADY EXISTS)
nano config/config.test.toml
```

---

## 📝 Monitoring Commands

### Real-Time Log Monitoring
```bash
# SSH into server
ssh root@YOUR_SERVER_IP

# Tail log in real-time
tail -f /opt/hfptm/logs/hfptm.log

# Filter for errors
tail -f /opt/hfptm/logs/hfptm.log | grep -i "error\|Error\|FAIL"

# Filter for arbitrage detections
tail -f /opt/hfptm/logs/hfptm.log | grep -i "Arbitrage detected"

# Filter for executions
tail -f /opt/hfptm/logs/hfptm.log | grep -i "Arbitrage executed"

# Filter for latency measurements
tail -f /opt/hfptm/logs/hfptm.log | grep -oP "in.*ms" | head -20
```

### System Resource Monitoring
```bash
# Check CPU usage
ssh root@YOUR_SERVER_IP "ps aux | grep hfptm"

# Check memory usage
ssh root@YOUR_SERVER_IP "free -h | grep Mem | grep hfptm"

# Check disk usage
ssh root@YOUR_SERVER_IP "df -h /opt/hfptm | grep -vE '^/dev'"

# Check network statistics
ssh root@YOUR_SERVER_IP "netstat -i | head -5"
```

### Automated Monitoring (Script Runs on Server)
```bash
# The deploy script already created this monitor script
# Monitor runs automatically and writes to /opt/hfptm/test-monitor.log

# Check monitoring output
cat /opt/hfptm/test-monitor.log
```

---

## 🔑 Testing Commands

### Phase 1: WebSocket Connection
```bash
# Monitor WebSocket connection logs
ssh root@YOUR_SERVER_IP "tail -f /opt/hfptm/logs/hfptm.log | grep WebSocket"

# Look for connection establishment
ssh root@YOUR_SERVER_IP "grep 'WebSocket connected' /opt/hfptm/logs/hfptm.log"

# Verify subscription success
ssh root@YOUR_SERVER_IP "grep 'Subscribing to' /opt/hfptm/logs/hfptm.log"
```

### Phase 2: Order Book Reception
```bash
# Check order book updates
ssh root@YOUR_SERVER_IP "tail -f /opt/hfptm/logs/hfptm.log | grep -i 'order book'"

# Count markets loaded
ssh root@YOUR_SERVER_IP "grep 'markets from Gamma' /opt/hfptm/logs/hfptm.log | tail -5"
```

### Phase 3: Arbitrage Detection
```bash
# Monitor detection rate
ssh root@YOUR_SERVER_IP "grep -i 'Arbitrage detected' /opt/hfptm/logs/hfptm.log | wc -l"

# Check for specific market
ssh root@YOUR_SERVER_IP "grep 'Arbitrage detected.*MARKET_ID' /opt/hfptm/logs/hfptm.log"

# Sample output:
# 🎯 Arbitrage #123: [market_id] (Binary) (2.5% edge, $10.50 profit, $50 position)
```

### Phase 4: Order Execution
```bash
# Check order creation
ssh root@YOUR_SERVER_IP "grep 'Created.*signed orders' /opt/hfptm/logs/hfptm.log"

# Check submission results
ssh root@YOUR_SERVER_IP "grep 'submit.*orders' /opt/hfptm/logs/hfptm.log | tail -10"

# Sample successful execution:
# ✅ Execution results: 2/2 orders filled, $20.00 filled, 180ms
```

### Phase 5: Latency Measurement
```bash
# Find latency measurements
ssh root@YOUR_SERVER_IP "grep 'Arbitrage executed.*in.*ms' /opt/hfptm/logs/hfptm.log | grep -oP 'in [0-9]*ms)'

# Calculate average
# Extract all times, compute avg

# Sample output:
# ✅ Arbitrage executed: [market] in 125ms (success: true)
# ✅ Arbitrage executed: [market] in 180ms (success: true)
```

### Phase 6: Risk Management
```bash
# Check risk rejections
ssh root@YOUR_SERVER_IP "grep -i 'Risk manager rejected' /opt/hfptm/logs/hfptm.log"

# Check position tracking
ssh root@YOUR_SERVER_IP "grep 'Recorded arbitrage' /opt/hfptm/logs/hfptm.log"

# Check daily PnL
ssh root@YOUR_SERVER_IP "grep 'daily_pnl' /opt/hfptm/logs/hfptm.log"
```

### Phase 7: Monitoring & Alerts
```bash
# Check metrics via dashboard
curl http://YOUR_SERVER_IP:3000/metrics | jq

# Check for Telegram alerts
ssh root@YOUR_SERVER_IP "grep -i 'telegram\|Alert' /opt/hfptm/logs/hfptm.log | tail -10"
```

---

## 🚨 Health Checks

### Critical Checks (Run These Every Hour)

**1. WebSocket Health**
```bash
# Check connection status
ssh root@YOUR_SERVER_IP "sudo systemctl is-active hfptm"

# Check logs for disconnects
tail -f /opt/hfptm/logs/hfptm.log | grep -i 'WebSocket closed' | wc -l
```

**Expected**: 0 disconnects per hour
**Threshold**: >5 disconnects/hour = concern

**2. Order Book Freshness**
```bash
# Check last update times
ssh root@YOUR_SERVER_IP "grep -i 'order book.*timestamp' /opt/hfptm/logs/hfptm.log | tail -10"

# Warning signs:
# - Timestamps >60 seconds old
# - "Stale book" errors in logs
```

**3. Detection Performance**
```bash
# Count detections in last hour
ssh root@YOUR_SERVER_IP "grep -i 'Arbitrage detected' /opt/hfptm/logs/hfptm.log | wc -l"

# Expected: 5-15 detections/hour (testing mode)
# Threshold: <3 detections/hour = investigate

**4. Execution Success Rate**
```bash
# Count successful executions
ssh root@YOUR_SERVER_IP "grep 'success: true' /opt/hfptm/logs/hfptm.log | wc -l

# Expected: 50-70% success rate (testing mode)
# Threshold: <50% = investigate

**5. Latency Spikes**
```bash
# Find high latency events
ssh root@YOUR_SERVER_IP "grep -oP 'in.*[3-9][0-9]*ms' /opt/hfptm/logs/hfptm.log | head -10"

# Warning signs:
# - Latency >200ms: Potential missed opportunities
# - Latency >500ms: System issue

---

## 📊 Expected Test Results

### With Conservative Test Config ($1K bankroll, 2.0% min edge, $50 max size)

| Metric | Expected Result | Pass/Fail Criteria |
|--------|---------------|-------------------|
| **WebSocket Stability** | Stable | No disconnects in 1 hour |
| **Message Processing** | Healthy | >1000 msg/s processing |
| **Order Book Reception** | Good | 10+ markets loaded, <5s stale books |
| **Arb Detection Rate** | Normal | 5-10 detections/hour expected |
| **Execution Success Rate** | Good | 50-70% success expected |
| **Avg Latency** | ~150ms | <200ms target met ✅ |
| **Risk Management** | Working | Limits enforced correctly |

### Success Indicators

✅ **ALL PASS** = Ready to consider live trading

**One or More WARNINGS = Proceed with caution, optimize first**

---

## 🔑 When Ready for Live Trading

### Update Configuration

```bash
# SSH into server
ssh root@YOUR_SERVER_IP
cd /opt/hfptm

# Switch to production config
cp config/config.toml config/config.prod.toml

# Edit for live settings
nano config/config.prod.toml
```

**Live Mode Settings**:
```toml
[trading]
bankroll = 1000              # Your full bankroll
max_arb_size = 200           # PRODUCTION size (10x test)
min_edge = 0.025             # 2.5% minimum profit
min_liquidity = 100          # $100 minimum liquidity

[risk]
max_exposure_per_market = 200  # Increased
max_exposure_per_event = 500    # Increased
max_concurrent_arbs = 10      # Increased
daily_loss_limit = 100        # Increased

[markets]
prioritize_categories = ["sports", "esports"]
```

### Restart Bot

```bash
# Stop test instance
sudo systemctl stop hfptm

# Start production instance
sudo systemctl start hfptm
```

---

## 📞 Monitoring During Live Trading

### Watchlist (First 30 Minutes)

1. **Latency Spikes**: Any execution >300ms
2. **Order Failures**: 3+ consecutive failed orders
3. **Detection Drops**: Sudden drop in detection rate >50%
4. **Risk Limit Breaches**: Loss limit approach or exposure limits hit
5. **WebSocket Disconnects**: Any unexpected reconnections

### Metrics to Track (Daily)

1. **Arbitrage Opportunities Detected**
   - Total count
   - Binary vs Multi-Outcome ratio
   - Average edge size

2. **Arbitrage Executions**
   - Success rate
   - Average position size
   - PnL (realized + unrealized)

3. **Latency**
   - P50/P99 detection-to-execution
   - Average WebSocket message time
   - Average API response time

4. **Risk Metrics**
   - Total exposure
   - Active arbitrages
   - Delta-neutral status
   - Daily PnL trend

---

## ⚙️ Emergency Procedures

### If Bot Crashes or Stops

**Immediate Actions**:
```bash
# 1. Check service status
sudo systemctl status hfptm

# 2. View crash logs
sudo journalctl -u hfptm -n 100 | tail -50

# 3. Restart service
sudo systemctl restart hfptm

# 4. Check system resources
ssh root@YOUR_SERVER_IP "free -h && df -h"

# 5. Alert via Telegram (if configured)
# Send message: "⚠️ Bot stopped! Please investigate"
```

### If Orders Keep Failing

**Diagnostic Steps**:
```bash
# 1. Check API credentials
ssh root@YOUR_SERVER_IP "cat /opt/hfptm/config/secrets.toml | grep api_key"

# 2. Check Polymarket status
curl https://clob.polymarket.com/ok

# 3. Verify USDC balance on Polymarket
# Check in UI at polymarket.com

# 4. Check logs for specific errors
grep -E "INVALID|FAILED|ERROR" /opt/hfptm/logs/hfptm.log | tail -20
```

### If Latency Increases Suddenly

**Possible Causes**:
- Network congestion
- Polymarket API issues
- High CPU usage (check with `htop`)
- Memory pressure (check with `free -h`)

**Solutions**:
1. Check Polymarket status: `curl https://clob.polymarket.com/ok`
2. Check network: `ping -c 20 ws-subscriptions-clob.polymarket.com`
3. Reduce concurrent operations: Lower `max_order_books` in config
4. Monitor resource usage: `ssh root@YOUR_SERVER_IP "htop"`
5. Check for memory leaks: `ssh root@YOUR_SERVER_IP "free -h"`

---

## 📈 Daily Check-in Procedure

### End of Each Day

**At End of Day 1-3**:
1. Review metrics from dashboard: `http://YOUR_SERVER_IP:3000/metrics`
2. Export trade history: `http://YOUR_SERVER_IP:3000/trades?limit=100`
3. Calculate daily PnL
4. Review alerts: `http://YOUR_SERVER_IP:3000/alerts?limit=50`
5. Check if any risk limits were breached

**Questions to Ask**:
- Was detection rate consistent with expectations?
- Did execution success rate meet targets?
- Was average latency acceptable?
- Did any risk management rules trigger?
- Were there any critical errors?

### Weekly Review (End of Week 1, 2, 4)

1. Download full trade history: `curl http://YOUR_SERVER_IP:3000/trades?limit=500`
2. Analyze best performing markets
3. Adjust min_edge based on capture rate
4. Review most common market categories

**Monthly Review (End of Month 1)**

1. Calculate monthly ROI
2. Analyze profit trends by market type
3. Decide if scaling is appropriate
4. Adjust bankroll size parameters

---

## 🎯 Scaling Strategy

### When to Increase Position Sizes

**Readiness Indicators**:
- ✅ Consistent 70-80% execution success rate for 3+ days
- ✅ Average latency <150ms for 3+ days
- ✅ Zero critical errors for 1+ weeks
- ✅ Daily PnL positive and growing
- ✅ Risk limits never exceeded

### Scaling Recommendations

**Week 2-3: (Growth Phase)**
```toml
[trading]
max_arb_size = 500           # Increase to $500
# Can target: $500-1000 per arb

[risk]
max_exposure_per_market = 500  # Increase to $500
max_exposure_per_event = 1500   # Increase to $1,500
max_concurrent_arbs = 20       # Increase to 20
daily_loss_limit = 200        # Increase to $200
```

**Week 4-6: (Mature Phase)**
```toml
[trading]
max_arb_size = 1000          # Maximum for serious trading
# Can target: $1,000-500 per arb

[risk]
max_exposure_per_market = 1000
max_exposure_per_event = 5000
max_concurrent_arbs = 50
daily_loss_limit = 500
```

**Week 7+: (Maximization)**
```toml
[trading]
max_arb_size = 2000          # Maximum aggressive
max_concurrent_arbs = 100
# Can target: $2,000,000+ per month
```

---

## 🔍 Verification Commands

### Pre-Live Final Verification

```bash
# 1. Check all configurations
ssh root@YOUR_SERVER_IP "cat /opt/hfptm/config/config.toml"

# 2. Verify service status
sudo systemctl status hfptm

# 3. Check recent logs (last 100 lines)
sudo journalctl -u hfptm -f -n 100

# 4. Check dashboard metrics
curl http://YOUR_SERVER_IP:3000/metrics | jq

# Expected values (conservative):
{
  "arb_detections_per_hour": 10-20,
  "arb_executions_per_hour": 7-14,
  "avg_latency_ms": 120-150,
  "p50_latency_ns": 110000,
  "p99_latency_ns": 180000,
  "total_pnl": 150.50,
  "active_positions": 8-12,
  "websocket_connected": true
}
```

---

## 📝 Configuration Examples

### Aggressive Mode (For $10K+ capital, experienced trader)

```toml
[trading]
bankroll = 10000             # Your bankroll
max_arb_size = 2000          # Max $2,000 per arb
min_edge = 0.015             # 1.5% min edge (more opportunities)
min_liquidity = 500          # $500 min liquidity (requires more capital)

[risk]
max_exposure_per_market = 2000  # $2K per market max
max_exposure_per_event = 5000   # $5K per event
max_concurrent_arbs = 50      # 50 concurrent
daily_loss_limit = 1000        # 1% daily limit (allows $100K/day loss)

[markets]
prioritize_categories = ["sports", "esports", "crypto", "politics"]
min_volume_24h = 5000
```

### Conservative Mode (For capital preservation)

```toml
[trading]
bankroll = 1000
max_arb_size = 100
min_edge = 0.025
min_liquidity = 100
max_exposure_per_market = 100
max_exposure_per_event = 500
max_concurrent_arbs = 10
daily_loss_limit = 100
```

---

## 🎯 Success Path Summary

### Testing → Live Progression

| Phase | Duration | Key Milestones |
|-------|--------|--------------|
| **1. Server Setup** | 30 min | VPS deployed, system configured |
| **2. WebSocket Test** | 10 min | Connection verified, message rate confirmed |
| **3. Order Book Test** | 10 min | 10+ markets receiving updates |
| **4. Arb Detection Test** | 30 min | Detection working, 5-15 opportunities/hour |
| **5. Order Execution Test** | 30 min | 70%+ success rate, <200ms latency |
| **6. 24-Hour Stability** | 24 hr | 99.5% uptime, zero crashes |
| **7. Risk Management Test** | 1 hr | Limits enforced correctly |

### GO LIVE! ✅

**Your bot has completed comprehensive testing and is ready for live trading.**

### Final Actions:
1. Update config for live settings (see Configuration Examples)
2. Restart bot with production config
3. Monitor first 5 live trades closely
4. Scale up gradually over coming weeks
5. Keep learning and optimizing

---

## 📞 Support Contact

### If Issues During Testing

**Documentation**:
- Full guide: `README.md`
- Testing guide: `TESTING_GUIDE.md`
- Technical details: `IMPLEMENTATION_SUMMARY.md`
- Quick reference: This file (`QUICK_REFERENCE.md`)

**Live Support**:
- GitHub Issues: Report bugs and feature requests
- Discord: https://discord.gg/polymarket
- Email: support contact (if available)

---

<div align="center">

**Your SG (Strasbourg) VPS is ready for comprehensive testing!** 🚀

<div align="center">

**Monitor your metrics: http://YOUR_SERVER_IP:3000**</div>
