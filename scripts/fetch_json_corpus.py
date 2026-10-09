#!/usr/bin/env python3
"""Fetch a JSON-rendering parity corpus from public XRPL nodes.

Writes xrpl/protocol/tests/fixtures/json_corpus.txt.gz with one record per
line:

    S <network> <ledger_index> <index-hex> <sle-hex>
    T <network> <ledger_index> <tx-hex> <meta-hex>

Ledger entries are collected per ledger-entry type (ledger_data `type`
filter) so rare types are covered; transactions come from recent validated
ledgers (capped per TransactionType). The data is public ledger state.

Usage: scripts/fetch_json_corpus.py [--ledgers N] [--per-type N]
"""

import argparse
import gzip
import json
import os
import sys
import time
import urllib.request

NETWORKS = {
    "mainnet": "https://s1.ripple.com:51234/",
    "testnet": "https://s.altnet.rippletest.net:51234/",
}

# Singleton ledger entries: Amendments, FeeSettings, NegativeUNL, LedgerHashes.
SINGLETONS = [
    "7DB0788C020F02780A673DC74757F23823FA3014C1866E72CC4CD8B226CD6EF4",
    "4BC50C9B0D8515D3EAAE1E74B29A95804346C491EE1A95BF25E4AAB854A6A651",
    "2E8A59AA9D3B5B186B0B9E0F62E6C02587CA74A4D778938E957B6357D364B244",
    "B4979A36CDC7F3D3D5C31A4EAE2AC7D7209DDA877588B9AFC66799692AB0D66B",
]


def rpc(url, method, params, retries=4):
    body = json.dumps({"method": method, "params": [params]}).encode()
    for attempt in range(retries):
        try:
            req = urllib.request.Request(
                url, data=body, headers={"Content-Type": "application/json"}
            )
            with urllib.request.urlopen(req, timeout=60) as resp:
                result = json.load(resp)["result"]
            if result.get("status") == "error":
                if result.get("error") in ("slowDown", "tooBusy"):
                    time.sleep(2 + attempt * 2)
                    continue
                return None
            return result
        except Exception as error:  # network hiccup: retry
            print(f"  {method} retry {attempt}: {error}", file=sys.stderr)
            time.sleep(2 + attempt * 2)
    return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ledgers", type=int, default=40)
    parser.add_argument("--per-type", type=int, default=60)
    parser.add_argument("--pages", type=int, default=2)
    parser.add_argument(
        "--out",
        default=os.path.join(
            os.path.dirname(__file__),
            "..",
            "xrpl/protocol/tests/fixtures/json_corpus.txt.gz",
        ),
    )
    args = parser.parse_args()

    lines = []
    for net, url in NETWORKS.items():
        validated = rpc(url, "ledger", {"ledger_index": "validated"})
        if not validated:
            print(f"{net}: unreachable, skipped", file=sys.stderr)
            continue
        seq = int(validated["ledger_index"])
        print(f"{net}: validated {seq}", file=sys.stderr)

        per_tx_type = {}
        entry_indices = {}
        for ledger in range(seq - args.ledgers + 1, seq + 1):
            binary = rpc(url, "ledger", {"ledger_index": ledger, "transactions": True,
                                         "expand": True, "binary": True})
            decoded = rpc(url, "ledger", {"ledger_index": ledger, "transactions": True,
                                          "expand": True})
            if not binary or not decoded:
                continue
            for tx in binary["ledger"].get("transactions", []):
                blob, meta = tx.get("tx_blob"), tx.get("meta")
                if not blob or not meta:
                    continue
                kind = blob[2:6]  # TransactionType header bytes
                if per_tx_type.get(kind, 0) >= args.per_type:
                    continue
                per_tx_type[kind] = per_tx_type.get(kind, 0) + 1
                lines.append(f"T {net} {ledger} {blob} {meta}")
            # Ledger entries touched by these transactions, by type.
            for tx in decoded["ledger"].get("transactions", []):
                meta = tx.get("metaData") or tx.get("meta") or {}
                for node in meta.get("AffectedNodes", []):
                    inner = next(iter(node.values()))
                    if "DeletedNode" in node:
                        continue
                    kind = inner.get("LedgerEntryType")
                    entry_indices.setdefault(kind, set()).add(inner.get("LedgerIndex"))
        print(f"  txs: {sum(per_tx_type.values())}", file=sys.stderr)

        # Natural mix of state entries.
        marker = None
        for _ in range(args.pages):
            params = {"ledger_index": seq, "binary": True, "limit": 2048}
            if marker:
                params["marker"] = marker
            result = rpc(url, "ledger_data", params)
            if not result:
                break
            for item in result.get("state", []):
                lines.append(f"S {net} {seq} {item['index']} {item['data']}")
            marker = result.get("marker")
            if not marker:
                break

        # Entries of every type seen in metadata, plus singletons.
        targets = []
        for kind, indices in sorted(entry_indices.items(), key=lambda kv: str(kv[0])):
            targets += sorted(i for i in indices if i)[: args.per_type]
        targets += SINGLETONS
        fetched = 0
        for index in targets:
            result = rpc(url, "ledger_entry", {"index": index, "binary": True,
                                               "ledger_index": seq})
            if result and result.get("node_binary"):
                lines.append(f"S {net} {seq} {result['index']} {result['node_binary']}")
                fetched += 1
        kinds = {k: min(len(v), args.per_type) for k, v in entry_indices.items()}
        print(f"  entries by type from metadata: {kinds}; fetched {fetched}", file=sys.stderr)

    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with gzip.open(args.out, "wt", compresslevel=9) as out:
        out.write("\n".join(lines) + "\n")
    print(f"wrote {len(lines)} records to {args.out}", file=sys.stderr)


if __name__ == "__main__":
    main()
