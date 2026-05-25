# xrpld Rust Node — Optimizations & Technical Notes

## Memory Optimizations

### 1. Leaf Node Memory (saves ~9GB during acquisition)
- `InnerNodeArrays` (child hashes + child pointers) boxed and set to `None` for leaf nodes
- Inner nodes: ~1,400 bytes (was 2,678)
- Leaf nodes: ~200 bytes (was 2,678)
- File: `xrpl/shamap/src/nodes/tree_node.rs`

### 2. UnsafeCell Hash Field (saves ~0.5GB)
- Replaced `RwLock<SHAMapHash>` with `UnsafeCell` for the hash field
- Lock-free, matching C++ behavior where hash is a plain field
- File: `xrpl/shamap/src/nodes/tree_node.rs`

### 3. SHAMapInnerNodeData Deduplication (saves ~1GB)
- Removed duplicate `child_hashes`/`children` from inner data struct
- Reads directly from `InnerNodeArrays`

## Sync & Acquisition Optimizations

### 4. NuDB Resume on Restart
- `fetch_node_data()` checks NuDB before making network requests
- Millions of nodes loaded in seconds on restart vs hours for fresh sync
- Implemented in `AccountStateSF`, `TransactionStateSF`, `LedgerSyncFilterStore`

### 5. Load Last Validated from DB on Startup
- Reads latest ledger from SQLite on startup
- Sets as validated/closed ledger so peers see node as synced
- Peers serve data immediately (vs refusing when node reports seq=1)

### 6. Full Below Cache Clear on Jump
- Clears `full_below_cache` when validated ledger jumps >1 seq or on startup
- Prevents stale entries from causing incomplete state trees

### 7. Sequential Ledger Advancement
- After initial sync, advances validated pointer sequentially (validated+1)
- Adjacent ledgers share 99.9% of state (tiny delta ~100-1000 nodes)
- Prevents gaps in the validated chain

### 8. Parallel Acquisition
- Multiple ledgers acquired simultaneously during initial sync
- All peers serve data in parallel (not round-robin)
- ~1500 nodes per response from each peer
- Shared NuDB means work on one ledger benefits all others

### 9. Clock Sync from Validator Sign Time
- Node clock synced from trusted validator sign_time
- Ensures `is_current()` check passes for validations
- No cap on adjustment (validators are authoritative on network time)

### 10. Dedicated Tokio Runtime for Consensus
- Consensus timer uses `tokio::time::sleep` (fresh per call)
- No persistent `Interval` that ties to a specific runtime
- Prevents "runtime is being shutdown" panics across runtime boundaries

## RPC Parity Status

All major RPC methods are implemented with full C++ parity:

| Category | Methods |
|----------|---------|
| Account | `account_info`, `account_lines`, `account_offers`, `account_objects`, `account_channels`, `account_currencies`, `account_nfts`, `account_tx` |
| Ledger | `ledger`, `ledger_closed`, `ledger_current`, `ledger_data`, `ledger_entry`, `ledger_header` |
| Transaction | `tx`, `submit`, `submit_multisigned`, `transaction_entry`, `simulate` |
| Order Book | `book_offers`, `book_changes` |
| Path | `path_find`, `ripple_path_find` |
| Subscription | `subscribe`, `unsubscribe` |
| Server | `server_info`, `server_state`, `fee`, `get_counts`, `peers`, `ping`, `random` |
| Signing | `sign`, `sign_for`, `wallet_propose`, `channel_authorize`, `channel_verify` |
| Admin | `stop`, `log_level`, `connect`, `peer_reservations_add/del/list`, `validator_info`, `feature` |
| NFT | `nft_buy_offers`, `nft_sell_offers` |

## Known Limitations

### Memory During Initial Acquisition
The node requires ~12-14GB RAM during initial mainnet sync because the full
SHAMap state tree (~50M nodes) is held in memory for verification. After sync
completes and the node is tracking the tip, memory drops to ~3-4GB. Future
optimization: stream nodes to disk during acquisition instead of holding the
full tree in memory.

### Transaction History
The `tx` command requires the transaction to be in a ledger within the node's
history window (`ledger_history` config). Full transaction history requires
`ledger_history = full` and significantly more disk space.
