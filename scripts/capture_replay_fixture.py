#!/usr/bin/env python3
"""Capture a full-parent-state replay fixture for a diverged testnet ledger.

Produces a fixture directory compatible with the
`sparse_production_close_matches_every_canonical_metadata_blob` harness
(xrpld/app/tests/production_replay_fixture.rs) in `full_parent_state` mode:

    <out>/manifest.json          # kind=manifest, parent/child headers, txns, fees, amendments
    <out>/state-0000.jsonl ...   # {kind:"sle", index, data} rows (full parent AccountState)

Usage:
    capture_replay_fixture.py --child-seq 21098395 --out /tmp/replay/21098395 \
        [--endpoint https://s.altnet.rippletest.net:51234]

Verification of the reconstructed parent state root is left to the Rust harness
(it asserts the SLEs hash to the canonical parent account_hash).
"""
import argparse
import json
import os
import sys
import time
import urllib.request

# Amendments singleton SLE index (ltAMENDMENTS): sha512half of 0x0066'ing key space.
AMENDMENTS_INDEX = "7DB0788C020F02780A673DC74757F23823FA3014C1866E72CC4CD8B226CD6EF4"
SHARD_ROWS = 20000  # keep individual shards modest


def rpc(endpoint, method, params, retries=4, timeout=40):
    body = json.dumps({"method": method, "params": [params]}).encode()
    last = None
    for attempt in range(retries):
        try:
            req = urllib.request.Request(
                endpoint, data=body, headers={"Content-Type": "application/json"}
            )
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                out = json.loads(resp.read())
            result = out.get("result", {})
            if result.get("status") == "success":
                return result
            last = result.get("error", result.get("status"))
        except Exception as exc:  # noqa: BLE001 - report and retry
            last = str(exc)
        time.sleep(1.5 * (attempt + 1))
    raise RuntimeError(f"rpc {method} failed after {retries} tries: {last}")


def header(endpoint, seq):
    r = rpc(endpoint, "ledger", {"ledger_index": int(seq), "transactions": False, "expand": False})
    return r["ledger"]


def child_with_txns(endpoint, seq):
    r = rpc(endpoint, "ledger", {"ledger_index": int(seq), "transactions": True, "expand": False})
    return r["ledger"]


def fetch_tx(endpoint, tx_hash):
    r = rpc(endpoint, "tx", {"transaction": tx_hash, "binary": True})
    # rippled binary tx: r["tx"] = tx blob, r["meta"] = metadata blob
    tx_hex = r.get("tx") or r.get("tx_blob")
    meta_hex = r.get("meta")
    if not tx_hex or not meta_hex:
        raise RuntimeError(f"tx {tx_hash}: missing tx/meta blob (keys={list(r)})")
    return tx_hex, meta_hex


def page_parent_state(endpoint, parent_seq, out_dir):
    """Page the full AccountState of the parent; write JSONL shards. Returns rows."""
    marker = None
    rows = 0
    shard_idx = 0
    shard = open(os.path.join(out_dir, f"state-{shard_idx:04d}.jsonl"), "w")
    try:
        while True:
            params = {"ledger_index": int(parent_seq), "binary": True, "limit": 2048}
            if marker is not None:
                params["marker"] = marker
            r = rpc(endpoint, "ledger_data", params)
            for entry in r.get("state", []):
                index = entry["index"]
                data = entry["data"]
                shard.write(json.dumps({"kind": "sle", "index": index, "data": data}) + "\n")
                rows += 1
                if rows % SHARD_ROWS == 0:
                    shard.close()
                    shard_idx += 1
                    shard = open(os.path.join(out_dir, f"state-{shard_idx:04d}.jsonl"), "w")
            marker = r.get("marker")
            sys.stderr.write(f"\r  paged {rows} SLEs...")
            sys.stderr.flush()
            if marker is None:
                break
    finally:
        shard.close()
    sys.stderr.write("\n")
    return rows


def fetch_amendments(endpoint, parent_seq):
    """Enabled amendment IDs from the Amendments singleton SLE (reliable JSON)."""
    r = rpc(endpoint, "ledger_entry",
            {"index": AMENDMENTS_INDEX, "ledger_index": int(parent_seq)})
    node = r.get("node", {})
    return [a.upper() for a in node.get("Amendments", [])]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--child-seq", type=int, required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--endpoint", default="https://s.altnet.rippletest.net:51234")
    ap.add_argument("--network-id", type=int, default=1)
    args = ap.parse_args()

    child_seq = args.child_seq
    parent_seq = child_seq - 1
    os.makedirs(args.out, exist_ok=True)

    sys.stderr.write(f"[1/4] headers for child {child_seq} / parent {parent_seq}\n")
    parent = header(args.endpoint, parent_seq)
    child = child_with_txns(args.endpoint, child_seq)

    sys.stderr.write(f"[2/4] paging parent {parent_seq} full state\n")
    rows = page_parent_state(args.endpoint, parent_seq, args.out)
    amendments = fetch_amendments(args.endpoint, parent_seq)

    sys.stderr.write(f"[3/4] fetching {len(child['transactions'])} child txns\n")
    txns = []
    # Preserve canonical order: fetch child expanded to keep order, then blobs per hash.
    ordered = child["transactions"]  # list of tx hashes (expand=false)
    for h in ordered:
        tx_hex, meta_hex = fetch_tx(args.endpoint, h)
        txns.append({"hash": h, "tx_hex": tx_hex, "metadata_hex": meta_hex})

    sys.stderr.write("[4/4] writing manifest.json\n")
    manifest = {
        "kind": "manifest",
        "full_parent_state": True,
        "network_id": args.network_id,
        "parent": {
            "ledger_index": int(parent["ledger_index"]),
            "ledger_hash": parent["ledger_hash"],
            "parent_hash": parent["parent_hash"],
            "account_hash": parent["account_hash"],
            "transaction_hash": parent["transaction_hash"],
            "total_coins": int(parent["total_coins"]),
            "close_time": int(parent["close_time"]),
            "parent_close_time": int(parent["parent_close_time"]),
            "close_time_resolution": int(parent["close_time_resolution"]),
            "close_flags": int(parent["close_flags"]),
        },
        "child": {
            "ledger_index": int(child["ledger_index"]),
            "close_time": int(child["close_time"]),
            "close_time_resolution": int(child["close_time_resolution"]),
            "close_flags": int(child["close_flags"]),
            "transaction_hash": child["transaction_hash"],
            "account_hash": child["account_hash"],
        },
        "fees": {"base": 10, "reserve": 10000000, "increment": 2000000},
        "enabled_amendments": amendments,
        "transactions": txns,
    }
    with open(os.path.join(args.out, "manifest.json"), "w") as fh:
        json.dump(manifest, fh, indent=2)

    sys.stderr.write(
        f"DONE: {rows} parent SLEs, {len(txns)} txns, {len(amendments)} amendments -> {args.out}\n"
    )
    print(json.dumps({"parent_seq": parent_seq, "child_seq": child_seq, "sle_rows": rows,
                      "txns": len(txns), "amendments": len(amendments),
                      "parent_account_hash": parent["account_hash"],
                      "child_account_hash": child["account_hash"],
                      "child_tx_hash": child["transaction_hash"]}))


if __name__ == "__main__":
    main()
