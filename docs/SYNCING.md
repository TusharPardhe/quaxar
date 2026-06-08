# Syncing

How quaxar synchronizes with the XRP Ledger network.

## Overview

Sync progresses through three phases:

1. **Genesis** — Connect to peers, download the latest validated ledger header
2. **Parallel Acquisition** — Download the full state tree and recent ledger history
3. **Tracking Tip** — Follow new validated ledgers in real-time

## Initial Sync

On first start, the node must download the complete state tree (~50 million nodes). This is the most resource-intensive phase.

What happens:
1. Node connects to peers and identifies the latest validated ledger
2. Requests state tree nodes in parallel from multiple peers
3. Stores nodes in NuDB as they arrive
4. Simultaneously fetches recent ledger headers and transactions
5. Once the full state is acquired, transitions to tracking the tip

## Hardware Impact

| Resource | During Acquisition | At Steady State |
|----------|-------------------|-----------------|
| RAM | 24–32 GB | 8–16 GB |
| Disk I/O | Very high (sustained writes) | Moderate |
| Disk Space | Grows ~1 GB/hour initially | ~200 MB/day |
| Network | 50–100 Mbps sustained | 5–20 Mbps |
| CPU | Moderate (hashing, decompression) | Low–moderate |

## Time Estimates

| Hardware | Estimated Sync Time |
|----------|-------------------|
| 8-core, 32 GB RAM, NVMe, 1 Gbps | 4–8 hours |
| 4-core, 32 GB RAM, NVMe, 100 Mbps | 8–16 hours |
| 4-core, 32 GB RAM, SATA SSD, 100 Mbps | 16–36 hours |

Times vary based on network conditions and peer availability.

## Monitoring Progress

### sync-status command

```bash
quaxar sync-status
```

Output:
```
State: acquiring
Progress: 34,521,000 / ~50,000,000 nodes (69%)
Rate: 45,000 nodes/sec
Peers providing data: 8
Estimated time remaining: 5h 42m
Current ledger: 95,000,000
```

### Log lines to watch

```
INFO  sync: State acquisition progress: 69% (34.5M / 50M nodes)
INFO  sync: Fetching ledger 95000000 state from 8 peers
INFO  sync: Acquisition complete, transitioning to tracking
INFO  sync: Fully synced, server_state=full
```

## Parallel Acquisition

The node downloads state from multiple ledgers simultaneously:

- Requests are distributed across all connected peers
- Each peer serves different portions of the state tree
- All data is stored in a shared NuDB instance
- Deduplication avoids re-downloading nodes shared between ledgers
- Back-pressure prevents overwhelming slower peers

## Full State

A node has "full" state when:
- The complete state tree for the latest validated ledger is stored locally
- The node can answer any `ledger_data` or `ledger_entry` query
- The node can validate new transactions against current state
- `server_state` reports `full`, `proposing`, or `validating`

## Troubleshooting

### Node stalls during sync

**Symptoms:** Progress stops, no new nodes downloaded.

**Causes:**
- All peers disconnected or unresponsive
- Network connectivity issues

**Fix:**
```bash
# Check peer count
quaxar peers

# Add fixed peers to config
# [ips_fixed]
# s1.ripple.com 51235
```

### Out of memory (OOM)

**Symptoms:** Process killed by OS, `dmesg` shows OOM killer.

**Causes:**
- Insufficient RAM for state acquisition cache
- `[node_size]` set too high for available memory

**Fix:**
- Ensure at least 32 GB RAM for mainnet
- Reduce `[node_size]` to `medium` or `large`
- Ensure no other memory-heavy processes running

### Slow sync

**Symptoms:** Progress rate below 10,000 nodes/sec.

**Causes:**
- Slow disk (SATA HDD)
- Limited bandwidth
- Few peers available

**Fix:**
- Use NVMe SSD
- Ensure 100+ Mbps network
- Add more `[ips_fixed]` entries for reliable peers
- Check `quaxar peers` — need at least 5–10 connected peers
