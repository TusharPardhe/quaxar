# RPC API

Quaxar serves XRPL JSON-RPC over configured HTTP and WebSocket ports. Method
names and result shapes follow the `rippled` API where implemented. Because
parity work is active, clients should feature-detect the fields they require
and pin the Quaxar version they validate against.

## Calling RPC

```bash
curl -fsS http://127.0.0.1:5005 \
  -H 'Content-Type: application/json' \
  -d '{"method":"server_info","params":[{}]}'

quaxar rpc server_info --raw
quaxar rpc ledger '{"ledger_index":"validated"}'
```

The CLI accepts a JSON object or array for parameters. HTTP requests normally
use `params: [{}]`. WebSocket also supports `subscribe` and `unsubscribe`.

## Access roles

Each request is assigned a user or admin role from its listener configuration,
source address, and gateway settings. Admin methods must not be exposed to
untrusted clients. Restrict admin scope to `127.0.0.1` and also bind or firewall
the host port appropriately; use an SSH tunnel for remote administration.

The following methods are wired into the current server dispatcher.

### Server and operations

| User methods | Admin methods |
|--------------|---------------|
| `ping`, `server_info`, `server_state`, `server_definitions`, `version`, `random`, `fee` | `connect`, `peers`, `stop`, `get_counts`, `print`, `consensus_info` |
| `manifest` | `log_level`, `logrotate`, `can_delete`, `ledger_cleaner`, `export_snapshot`, `snapshot_status` |
| `wallet_propose` | `fetch_info`, `blacklist` |
| `subscribe`, `unsubscribe` | `peer_reservations_add`, `peer_reservations_del`, `peer_reservations_list` |

`logrotate` is the currently dispatched RPC spelling; the compatibility
registry also reserves `log_rotate`. The dispatched method currently returns
success without rotating a file because the runtime does not own one. The CLI
command is `quaxar log-rotate`.

### Ledgers and transactions

| Ledger | Transaction |
|--------|-------------|
| `ledger`, `ledger_closed`, `ledger_current`, `ledger_entry`, `ledger_data`, `ledger_header` | `tx`, `transaction_entry`, `tx_history` |
| `ledger_accept` (admin/standalone), `ledger_request` (admin) | `submit`, `submit_multisigned`, `simulate`, `sign`, `sign_for` |

### Accounts, paths, books, and objects

`account_info`, `account_lines`, `account_tx`, `account_channels`,
`account_currencies`, `account_nfts`, `account_objects`, `account_offers`,
`owner_info`, `gateway_balances`, `deposit_authorized`, `book_offers`,
`book_changes`, `path_find`, `ripple_path_find`, `no_ripple_check`,
`noripple_check`, `nft_buy_offers`, `nft_sell_offers`, `amm_info`,
`vault_info`, and `get_aggregate_price`.

### Validators, amendments, and relay

| User methods | Admin methods |
|--------------|---------------|
| `channel_authorize`, `channel_verify` | `validator_info`, `validators`, `validator_list_sites`, `unl_list` |
| `tx_reduce_relay` | `feature`, `validation_create` |

The compile-time RPC registry can contain compatibility names before every
behavioral edge has parity. A registered name is not by itself a production
compatibility guarantee; integration tests and the server dispatcher are the
authoritative implementation boundary.

## Common requests

### Server and synchronization state

```json
{"method":"server_info","params":[{}]}
```

Important fields include `server_state`, `validated_ledger`,
`complete_ledgers`, `peers`, `pubkey_validator`, `validator_list`, `load`,
`fetch_pack`, and runtime counters. Human and non-human response modes can
represent some values differently.

```json
{"method":"fetch_info","params":[{}]}
```

`fetch_info` is admin-only and reports active acquisition state. Pair it with
`get_counts` when diagnosing SHAMap cache or NodeStore behavior.

### Account lookup

```json
{
  "method": "account_info",
  "params": [{
    "account": "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh",
    "ledger_index": "validated"
  }]
}
```

### Snapshot export

Snapshot export is asynchronous and admin-only:

```json
{
  "method": "export_snapshot",
  "params": [{"output":"/var/lib/quaxar/snapshots/bootstrap.xrpls"}]
}
```

Only one export can run at a time. Poll its in-memory job state:

```json
{"method":"snapshot_status","params":[{}]}
```

The state is `idle`, `running`, `completed`, or `failed`. A completed result
includes the output path, ledger sequence, and file size; a failed result
includes an error. Restarting the node resets this status to `idle`.

### Runtime log level

```json
{"method":"log_level","params":[{"severity":"debug"}]}
```

For one partition:

```json
{
  "method": "log_level",
  "params": [{"severity":"debug","partition":"consensus"}]
}
```

Restore the previous level after diagnostics to avoid excess log volume.
Supplying a severity updates the runtime filter. The no-parameter get path
currently returns an empty compatibility `levels` object rather than the live
filter state.

## Errors

RPC errors are returned inside `result` with `status: "error"`, a symbolic
`error`, numeric `error_code` where applicable, and `error_message`. HTTP
success therefore does not necessarily mean the RPC operation succeeded.

```json
{
  "result": {
    "error": "noPermission",
    "error_code": 69,
    "error_message": "You don't have permission for this command.",
    "status": "error"
  }
}
```

Clients should branch on `result.status` and symbolic error names rather than
matching human-readable messages.
