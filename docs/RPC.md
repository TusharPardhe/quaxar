# RPC API Reference

## Connection

Send JSON-RPC requests via HTTP POST to the configured admin or public port:

```bash
curl -s http://127.0.0.1:5005 \
  -H "Content-Type: application/json" \
  -d '{"method": "server_info", "params": [{}]}'
```

## Supported Methods

### Server

| Method | Description |
|--------|-------------|
| `server_info` | Server status, version, ledger range, peer count |
| `server_state` | Detailed server state for monitoring |
| `fee` | Current transaction fee estimates |
| `peers` | Connected peer list with latency and version |
| `log_level` | Get or set log verbosity |
| `get_counts` | Internal object counts and memory usage |
| `stop` | Graceful server shutdown (admin only) |
| `connect` | Connect to a specific peer (admin only) |

### Ledger

| Method | Description |
|--------|-------------|
| `ledger` | Fetch a ledger header by index or hash |
| `ledger_data` | Paginated state tree entries for a ledger |
| `ledger_entry` | Fetch a specific ledger object by ID |

### Account

| Method | Description |
|--------|-------------|
| `account_info` | Account root object (balance, sequence, flags) |
| `account_lines` | Trust lines for an account |
| `account_offers` | Open offers for an account |
| `account_objects` | All objects owned by an account |
| `account_tx` | Transaction history for an account |

### Transactions

| Method | Description |
|--------|-------------|
| `tx` | Look up a transaction by hash |
| `submit` | Submit a signed transaction |
| `sign` | Sign a transaction (admin only) |

### Order Book

| Method | Description |
|--------|-------------|
| `book_offers` | Offers in an order book |
| `gateway_balances` | Obligations and assets for a gateway |

### Peer Management

| Method | Description |
|--------|-------------|
| `peer_reservations_add` | Reserve a peer slot (admin only) |
| `peer_reservations_del` | Remove a peer reservation (admin only) |
| `peer_reservations_list` | List peer reservations (admin only) |

### Validators & Amendments

| Method | Description |
|--------|-------------|
| `validator_info` | This node's validator identity and status |
| `feature` | List or vote on amendments |

### Simulation

| Method | Description |
|--------|-------------|
| `simulate` | Simulate a transaction without submitting |

## Examples

### server_info

**Request:**
```json
{
  "method": "server_info",
  "params": [{}]
}
```

**Response:**
```json
{
  "result": {
    "info": {
      "build_version": "2.0.0-rust",
      "server_state": "full",
      "complete_ledgers": "32570-95000000",
      "peers": 21,
      "uptime": 86400,
      "validated_ledger": {
        "seq": 95000000,
        "hash": "ABC123...",
        "close_time": 780000000
      }
    },
    "status": "success"
  }
}
```

### account_info

**Request:**
```json
{
  "method": "account_info",
  "params": [{
    "account": "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh",
    "ledger_index": "validated"
  }]
}
```

**Response:**
```json
{
  "result": {
    "account_data": {
      "Account": "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh",
      "Balance": "100000000000",
      "Sequence": 1,
      "Flags": 0
    },
    "ledger_index": 95000000,
    "status": "success"
  }
}
```

### fee

**Request:**
```json
{
  "method": "fee",
  "params": [{}]
}
```

**Response:**
```json
{
  "result": {
    "current_ledger_size": "42",
    "current_queue_size": "0",
    "drops": {
      "base_fee": "10",
      "median_fee": "5000",
      "minimum_fee": "10",
      "open_ledger_fee": "10"
    },
    "expected_ledger_size": "200",
    "ledger_current_index": 95000001,
    "status": "success"
  }
}
```

## Error Format

Errors return a `status` of `"error"` with an error code and message:

```json
{
  "result": {
    "error": "actNotFound",
    "error_code": 19,
    "error_message": "Account not found.",
    "status": "error"
  }
}
```

Common error codes:

| Code | Name | Meaning |
|------|------|---------|
| 19 | `actNotFound` | Account does not exist |
| 27 | `lgrNotFound` | Ledger not available |
| 29 | `invalidParams` | Malformed request parameters |
| 69 | `noPermission` | Admin-only method called on public port |
