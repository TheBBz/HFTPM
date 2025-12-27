# 🚀 READY TO GO LIVE - Final Preparation Guide

> **Complete checklist to verify testing is complete and switch to production**  
> **For SG (Strasbourg) VPS with ~30-40ms latency to Polymarket**

---

## ✅ PRE-FLIGHT CHECKLIST (Complete All Before Switching to Live)

### Infrastructure Readiness
- [ ] SG (Strasbourg) VPS deployed and accessible (`ssh root@SERVER_IP`)
- [ ] System service running (`sudo systemctl status hfptm`)
- [ ] Dashboard accessible at `http://SERVER_IP:3000`
- [ ] Monitoring script running (`/opt/hfptm/monitor-test.sh`)
- [ ] Log files being written to `/opt/hfptm/logs/`

### Configuration Readiness
- [ ] Production config created (`config/config.prod.toml`)
- [ ] Live trading parameters set:
  - Bankroll: $1,000 (your target)
  - Max position size: $500 (production)
  - Min edge: 2.5% (balanced)
  - Risk limits increased appropriately
- [ ] Market categories prioritized (sports, esports)
- [ ] Telegram alerts configured (if desired)
- [ ] API credentials verified

### Security Readiness
- [ ] `config/secrets.toml` contains all required credentials
- [ ] Private key is correct format (0x... 64 hex chars)
- [ ] Funder address matches Polymarket wallet
- [ ] No secrets in git repository (`.gitignore` active)
- [ ] SSH keys set up securely (only you have access)
- [ ] Firewall configured correctly (allow 443/tcp)

### Testing Readiness

#### Phase 1: WebSocket (✅ or 🔄)
- [ ] WebSocket connects within 30 seconds
- [ ] Receives order book updates for 10+ markets
- [ ] Message rate >1000 msg/s
- [ ] No connection errors for 1 hour

**Verification Command**:
```bash
ssh root@YOUR_SERVER_IP "tail -f /opt/hfptm/logs/hfptm.log | grep -i 'WebSocket connected' | head -5"
```

**Expected Output**: ✅ "WebSocket connected to wss://ws-subscriptions-clob.polymarket.com/ws/market"

#### Phase 2: Order Book (✅ or 🔄)
- [ ] Receives snapshots for 500+ markets
- [ ] Order book updates <5 seconds old (not stale)
- [ ] Multiple markets updating concurrently

**Verification Command**:
```bash
ssh root@YOUR_SERVER_IP "tail -f /opt/hfptm/logs/hfptm.log | grep -i 'Updated order book' | tail -10"
```

**Expected Output**: Multiple lines showing order book updates

#### Phase 3: Arbitrage Detection (✅ or 🔄)
- [ ] Detection rate: 5-15 opportunities/hour (sports focus)
- [ ] Both binary and multi-outcome detection working
- [ ] Edges fall in expected range (2.0-5.0%)

**Verification Commands**:
```bash
# Count detections in last hour
ssh root@YOUR_SERVER_IP "grep -i 'Arbitrage detected' /opt/hfptm/logs/hfptm.log | wc -l"

# Expected: 5-15 detections
```

#### Phase 4: Order Execution (✅ or 🔄)
- [ ] Order creation working (no signing errors)
- [ ] Orders submitted successfully (no API failures)
- [ ] 50-70% success rate
- [ ] Average execution time <200ms

**Verification Commands**:
```bash
# Check submission success rate
ssh root@YOUR_SERVER_IP "grep 'success: true' /opt/hfptm/logs/hfptm.log | wc -l"

# Expected: 70-90% success rate for 5-10 attempts
```

#### Phase 5: Latency (✅ or 🔄)
- [ ] End-to-end latency <200ms (detection → execution)
- [ ] P50 latency: ~110ms
- [ ] P99 latency: ~180ms

**Verification Commands**:
```bash
# Get recent measurements
ssh root@YOUR_SERVER_IP "grep -oP 'Arbitrage executed.*in.*ms' /opt/hfptm/logs/hfptm.log | grep -oP 'in [0-9]*ms' | head -10"

# Sample output:
# ✅ Arbitrage executed: [market] in 125ms
# ✅ Arbitrage executed: [market] in 180ms
```

Expected: P50 <150ms, P99 <200ms, Average ~150ms ✅

#### Phase 6: Risk Management (✅ or 🔄)
- [ ] Risk manager actively enforcing limits
- [ ] Position tracking working correctly
- [ ] No exposure limits breached
- [ ] Daily PnL being tracked

**Verification Commands**:
```bash
# Check risk rejections
ssh root@YOUR_SERVER_IP "grep -i 'Risk manager rejected' /opt/hfptm/logs/hfptm.log"

# Check exposure
ssh root@YOUR_SERVER_IP "grep -i 'Active arbitrages: [0-9]+' /opt/hfptm/logs/hfptm.log | head -10"
```

Expected: Active arbitrages <10, exposure <$500, delta-neutral

#### Phase 7: Monitoring (✅ or 🔄)
- [ ] Dashboard accessible at `http://SERVER_IP:3000`
- [ ] Metrics endpoint returning valid JSON
- [ ] Trade history accessible
- [ ] Alerts generated (if configured)

**Verification Commands**:
```bash
# Check metrics
curl http://SERVER_IP:3000/metrics | jq

# Check alerts
curl http://SERVER_IP:3000/alerts?limit=10
```

Expected: All endpoints working

---

## 🚨 FAILURE MODE HANDLING

### If Any Phase Shows 🔄 (Needs Attention)

#### WebSocket Connection Issues

**Symptoms**:
- ❌ Multiple connection errors
- ❌ Frequent reconnections
- ❌ Connection time >60 seconds
- ❌ "Failed to parse WebSocket message" errors

**Immediate Actions**:
```bash
# 1. Check network status from server
ssh root@YOUR_SERVER_IP "ping -c 20 ws-subscriptions-clob.polymarket.com"

# 2. Check Polymarket status
curl https://clob.polymarket.com/ok

# 3. Check system resources
ssh root@YOUR_SERVER_IP "htop"

# 4. Check for connection saturation
ssh root@YOUR_SERVER_IP "netstat -i | head -10"

# 5. Review full logs for errors
sudo journalctl -u hfptm -n 100 | tail -50
```

#### Order Book Issues

**Symptoms**:
- ❌ No order book updates for >1 minute
- ❌ "Order book is stale" warnings
- ❌ Very few markets updating
- ❌ JSON parse errors

**Immediate Actions**:
```bash
# 1. Check Gamma API connectivity
curl https://gamma-api.polymarket.com

# 2. Verify market filtering
ssh root@YOUR_SERVER_IP "grep 'prioritize_categories' /opt/hfptm/config/config.prod.toml"

# 3. Check if markets are being filtered out
ssh root@YOUR_SERVER_IP "grep 'Skipping market' /opt/hfptm/logs/hfptm.log | wc -l"
```

#### Arbitrage Detection Issues

**Symptoms**:
- ❌ Zero detections in >30 minutes
- ❌ All edges outside expected range (>10%)
- ❌ Detection errors frequent

**Investigation Steps**:
```bash
# 1. Check if markets are active
# 2. Verify WebSocket subscription
# 3. Review market activity on Polymarket UI
# 4. Check if filtering is too restrictive
```

**Potential Solutions**:
1. Lower `min_edge` to 0.020 (1.5% threshold)
2. Remove categories from `prioritize_categories`
3. Check if network issues causing message drops
4. Verify order book freshness

#### Order Execution Issues

**Symptoms**:
- ❌ 0% order success rate
- ❌ Frequent "Order submission failed" errors
- ❌ High latency (>300ms)
- ❌ Partial fills only

**Immediate Actions**:
```bash
# 1. Check API credentials
ssh root@YOUR_SERVER_IP "cat /opt/hfptm/config/secrets.toml | grep api_key"

# 2. Verify USDC balance
curl https://clob.polymarket.com/balance

# 3. Check error logs
grep -E "FAILED|ERROR" /opt/hfptm/logs/hfptm.log | tail -20

# 4. Test latency
ping -c 20 ws-subscriptions-clob.polymarket.com

# 5. Verify system resources
ssh root@YOUR_SERVER_IP "htop"
```

**Potential Solutions**:
1. Verify API credentials are correct
2. Deposit more USDC if balance low
3. Check Polymarket API status
4. Consider reducing concurrent operations
5. Check if server needs scaling (CPU/Memory)

#### Risk Management Issues

**Symptoms**:
- ❌ Risk limits frequently breached
- ❌ Daily loss limit reached
- ❌ Inventory drift too large

**Immediate Actions**:
```bash
# STOP TRADING IMMEDIATELY
ssh root@YOUR_SERVER_IP "sudo systemctl stop hfptm"

# Investigate:
# 1. Review recent trades
ssh root@YOUR_SERVER_IP "tail -f /opt/hfptm/logs/hfptm.log | grep -i 'Arbitrage executed'"

# 2. Check what went wrong
# 3. Review configuration settings
ssh root@YOUR_SERVER_IP "cat /opt/hfptm/config/config.prod.toml"
```

**Corrective Actions**:
1. Reduce position sizes
2. Lower `max_concurrent_arbs` (from 10 to 5)
3. Increase daily loss limit
4. Verify market selection criteria

#### High Latency Issues

**Symptoms**:
- ❌ Average latency >300ms
- ❌ Frequent latency spikes (>500ms)
- ❌ P99 latency >500ms

**Immediate Actions**:
```bash
# 1. Check network latency
ping -c 20 ws-subscriptions-clob.polymarket.com

# 2. Check CPU usage
ssh root@YOUR_SERVER_IP "htop"

# 3. Check for memory leaks
ssh root@YOUR_SERVER_IP "free -h"

# 4. Review system configuration
ssh root@YOUR_SERVER_IP "cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor"
```

**Potential Solutions**:
1. Contact OVHcloud support if network issues
2. Consider upgrading to better location (DigitalOcean Amsterdam3)
3. Reduce concurrent operations
4. Scale up server (more cores, more RAM)
5. Optimize system configuration

---

## ✅ ALL CHECKLIST PASSED - GO LIVE!

### Final Verification Commands

```bash
# 1. Get comprehensive metrics
curl http://SERVER_IP:3000/metrics | jq

# Expected results (conservative production settings):
{
  "arb_detections_per_hour": "8-12",
  "arb_executions_per_hour": "5-6",
  "avg_latency_ms": "120-180",
  "p50_latency_ns": "110000",
  "p99_latency_ns": "180000",
  "total_pnl": "150.00-200",
  "active_positions": "8-15",
  "websocket_connected": true
}

# 2. Check system health
ssh root@YOUR_SERVER_IP "htop"

# 3. Review recent alerts
curl http://SERVER_IP:3000/alerts?limit=20

# 4. Verify service stability
sudo journalctl -u hfptm | grep -i "active" | tail -5
```

---

## 🔄 SWITCHING TO LIVE MODE

### Step 1: Stop Test Mode

```bash
# SSH into server
ssh root@YOUR_SERVER_IP

# Stop current test instance
sudo systemctl stop hfptm
```

### Step 2: Update Configuration

```bash
# SSH into server
ssh root@YOUR_SERVER_IP
cd /opt/hfptm

# Create production config
cp config/config.prod.toml

# Edit production config
nano config/config.prod.toml
```

**Add/modify these parameters**:

```toml
[trading]
bankroll = 1000              # Your target bankroll
max_arb_size = 200           # Production size
min_edge = 0.025             # 2.5% minimum profit (balanced)
min_liquidity = 100          # $100 min liquidity (requires more capital)

[risk]
max_exposure_per_market = 200  # Production exposure
max_exposure_per_event = 500    # Production exposure
max_concurrent_arbs = 10      # Production concurrent
daily_loss_limit = 100        # 1% daily limit (allows $10K/day loss)

[markets]
prioritize_categories = ["sports", "esports", "crypto"]
min_volume_24h = 5000
```

### Step 3: Start Production Instance

```bash
# Restart with production config
sudo systemctl restart hfptm
```

### Step 4: Monitor First 30 Minutes

```bash
# Watch logs in real-time
ssh root@YOUR_SERVER_IP "tail -f /opt/hfptm/logs/hfptm.log"

# Check for successful first live trade:
# - Should see: "Arbitrage executed: [market_id] (2.0-5% edge, $200 profit)"
# - Should execute in <200ms
# - Should see: "✅ Execution results: 2/2 orders filled"

# Monitor for issues:
# - Any latency spikes?
# - Order failures?
# - Risk limit breaches?
```

### Step 5: Evaluate Performance

**After 1 Hour**:
- Check metrics: `curl http://SERVER_IP:3000/metrics`
- Verify detection rate: Should be 8-12/hour
- Verify execution rate: Should be 50-70%
- Verify latency: Should be <200ms

**After 3 Hours**:
- Review metrics again
- Check detection rate: Should be increasing
- Verify execution rate: Should be improving
- Check latency: Should be stable

### Step 6: Scale Up (After 1-2 Days)

If all good:
- Increase max_arb_size to $500-1000
- Increase max_concurrent_arbs to 20

If all excellent:
- Increase max_arb_size to $2000 (maximum)
- Increase max_concurrent_arbs to 50

---

## 🎯 Final Success Criteria

### MUST PASS BEFORE GOING LIVE WITH REAL MONEY:

**Infrastructure:**
- [ ] WebSocket connects within 30 seconds
- [ ] Receives order books for 10+ markets
- [ ] Bot service stable (99%+ uptime)
- [ ] Dashboard accessible

**Functionality:**
- [ ] Arbitrage detection working (5-15+/hour)
- [ ] Order execution working (50-70%+ success)
- [ ] End-to-end latency <200ms
- [ ] Risk limits enforcing correctly

**Testing:**
- [ ] 24+ hour stability verified
- [ ] All 9 testing phases completed
- [ ] No critical issues detected

**Configuration:**
- [ ] Production config prepared with appropriate risk limits
- [ ] Telegram alerts configured (optional)
- [ ] You understand all trading risks

**Performance:**
- [ ] Detection rate matches expectations
- [ ] Execution rate meets targets
- [ ] Latency meets target (<200ms)
- [ ] System resources sufficient

---

## 🎉 Congratulations!

**Your bot has completed comprehensive testing!**

**You are ready to go live with real trading when you're comfortable.**

### Next Steps:

1. **Review this guide** and understand success criteria
2. **Follow final verification commands** above
3. **Configure production settings** (see Configuration Examples)
4. **Switch bot to live mode** (see "SWITCHING TO LIVE MODE")
5. **Monitor first 5-30 minutes closely**
6. **Scale up gradually** as you gain confidence
7. **Reinvest profits** for compound growth

---

## 📞 Emergency Contacts

### If Issues During Live Trading

**Critical Situations:**
- Bot stops unexpectedly
- Risk limits breached (daily loss limit)
- Latency spikes >500ms
- API fails repeatedly

**Actions:**
1. Check service: `sudo systemctl status hfptm`
2. Check logs: `sudo journalctl -u hfptm -n 50`
3. Review dashboard: `http://SERVER_IP:3000/metrics`
4. Stop trading if losses escalating: `sudo systemctl stop hfptm`
5. Contact support if needed

### Support Resources
- Full documentation: `README.md`
- Testing guide: `TESTING_GUIDE.md`
- Technical details: `IMPLEMENTATION_SUMMARY.md`
- Quick reference: `QUICK_REFERENCE.md`
- Server deployment: `deploy-ovhcloud.sh`

---

## 🚨 IMPORTANT REMINDERS

⚠️ **Start Small**: Begin with $100-200 position sizes
⚠️ **Scale Gradually**: Only increase after consistent success
⚠️ **Monitor Closely**: Watch for issues in first week
⚠️ **Don't Panic**: If latency increases temporarily, it will normalize
⚠️ **Risk Management**: Never disable risk limits
⚠️ **Stay Realistic**: Expect some missed opportunities
⚠️ **Learn From Data**: Analyze what works and what doesn't
⚠️ **Keep Learning**: Adjust parameters based on results

---

<div align="center">

**✅ YOU'RE READY FOR LIVE TRADING!** ⭐

<div align="center">

**Monitor your bot: http://SERVER_IP:3000/metrics**</div>
