#!/usr/bin/env python3
"""Capture a sparse replay fixture for one network ledger.

Writes JSONL for xrpld/app/tests/production_replay_fixture.rs
(sparse_production_close_matches_every_canonical_metadata_blob):
line 1 = manifest, following lines = parent SLEs {"kind":"sle","index","data"}.

Usage: capture_sparse_replay.py <child_seq> <out.jsonl> [extra_index ...]
"""
import json, sys, urllib.request, hashlib

RPC = "https://s.altnet.rippletest.net:51234"
NETWORK_ID = 1


def rpc(method, **params):
    req = urllib.request.Request(
        RPC, json.dumps({"method": method, "params": [params]}).encode(),
        {"Content-Type": "application/json"})
    out = json.load(urllib.request.urlopen(req, timeout=60))["result"]
    if out.get("status") == "error":
        raise RuntimeError(f"{method} {params}: {out.get('error')}")
    return out


B58 = "rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz"


def account_id(addr):
    n = 0
    for c in addr:
        n = n * 58 + B58.index(c)
    raw = n.to_bytes(25, "big")
    return raw[1:21]


def sha512h(b):
    return hashlib.sha512(b).digest()[:32].hex().upper()


def account_key(addr):
    return sha512h(b"\x00a" + account_id(addr))


def owner_dir_key(addr):
    return sha512h(b"\x00O" + account_id(addr))


def dir_page_key(root, page):
    return root if page == 0 else sha512h(b"\x00d" + bytes.fromhex(root) + page.to_bytes(8, "big"))


SINGLETONS = [
    "7DB0788C020F02780A673DC74757F23823FA3014C1866E72CC4CD8B226CD6EF4",  # Amendments
    "4BC50C9B0D8515D3EAAE1E74B29A95804346C491EE1A95BF25E4AAB854A6A651",  # FeeSettings
    "B4979A36CDC7F3D3D5C31A4EAE2AC7D7209DDA877588B9AFC66799692AB0D66B",  # LedgerHashes
    "2E8A59AA9D3B5B186B0B9E0F62E6C02587CA74A4D778938E957B6357D364B244",  # NegativeUNL
]


def main():
    child_seq = int(sys.argv[1]); out_path = sys.argv[2]; extra = sys.argv[3:]
    parent_seq = child_seq - 1
    child = rpc("ledger", ledger_index=child_seq, transactions=True, expand=True)["ledger"]
    child_bin = rpc("ledger", ledger_index=child_seq, transactions=True, expand=True, binary=True)["ledger"]
    parent = rpc("ledger", ledger_index=parent_seq)["ledger"]

    want = set(SINGLETONS) | set(extra)
    accounts = set()
    # long skip list for this 65536-block
    want.add(sha512h(b"\x00s" + (parent_seq >> 16).to_bytes(4, "big")))

    def collect_accounts(obj):
        if isinstance(obj, dict):
            for k, v in obj.items():
                if isinstance(v, str) and v.startswith("r") and 25 <= len(v) <= 35 and k in (
                        "Account", "Destination", "Owner", "Issuer", "issuer", "RegularKey",
                        "Delegate", "Counterparty", "Authorize", "Unauthorize", "Subject", "Borrower"):
                    accounts.add(v)
                collect_accounts(v)
        elif isinstance(obj, list):
            for v in obj:
                collect_accounts(v)

    for t in child["transactions"]:
        collect_accounts(t)
        for n in t["metaData"]["AffectedNodes"]:
            kind = next(iter(n)); v = n[kind]
            if kind != "CreatedNode":
                want.add(v["LedgerIndex"])
            collect_accounts(v)

    for a in accounts:
        want.add(account_key(a))
        root = owner_dir_key(a)
        want.add(root)
        # NFToken pages: keys are AccountID || low 96 bits of the token, so
        # enumerate the account's whole page range with a crafted marker.
        prefix = account_id(a).hex().upper()
        marker = prefix + "0" * 24
        while marker:
            try:
                page = rpc("ledger_data", ledger_index=parent_seq, marker=marker,
                           limit=64, type="nft_page", binary=True)
            except RuntimeError:
                break
            rows_in = [r for r in page.get("state", []) if r["index"].startswith(prefix)]
            for r in rows_in:
                want.add(r["index"])
            nxt = page.get("marker")
            marker = nxt if (nxt and nxt.startswith(prefix) and len(rows_in) == len(page.get("state", []))) else None

    # NFToken buy/sell offer directories for every referenced NFTokenID.
    def collect_nft_ids(obj, out):
        if isinstance(obj, dict):
            for k, v in obj.items():
                if k == "NFTokenID" and isinstance(v, str) and len(v) == 64:
                    out.add(v)
                collect_nft_ids(v, out)
        elif isinstance(obj, list):
            for v in obj:
                collect_nft_ids(v, out)
    nft_ids = set()
    collect_nft_ids(child["transactions"], nft_ids)
    for nid in nft_ids:
        for ns in (b"\x00h", b"\x00i"):  # NFTOKEN_BUY_OFFERS, NFTOKEN_SELL_OFFERS
            want.add(sha512h(ns + bytes.fromhex(nid)))

    rows = {}

    def fetch(idx):
        try:
            r = rpc("ledger_entry", index=idx, ledger_index=parent_seq, binary=True)
            return r["node_binary"]
        except RuntimeError:
            return None

    pending = list(want)
    seen = set()
    while pending:
        idx = pending.pop()
        if idx in seen:
            continue
        seen.add(idx)
        data = fetch(idx)
        if data is None:
            continue
        rows[idx] = data
        # follow owner-directory pages
        try:
            j = rpc("ledger_entry", index=idx, ledger_index=parent_seq)["node"]
        except RuntimeError:
            continue
        if j.get("LedgerEntryType") == "DirectoryNode":
            root = j.get("RootIndex", idx)
            for page_field in ("IndexNext", "IndexPrevious"):
                p = int(j.get(page_field, "0"), 16)
                if p:
                    pending.append(dir_page_key(root, p))
            # owner directories: include every owned entry (reads during deletes)
            if "Owner" in j or "NFTokenID" in j:
                pending.extend(j.get("Indexes", []))

    amendments = rpc("ledger_entry", index=SINGLETONS[0], ledger_index=parent_seq)["node"]["Amendments"]
    fee = rpc("ledger_entry", index=SINGLETONS[1], ledger_index=parent_seq)["node"]

    def drops(v):
        return int(v) if isinstance(v, (int, str)) else 0
    fees = {
        "base": drops(fee.get("BaseFeeDrops", fee.get("BaseFee", 10))),
        "reserve": drops(fee.get("ReserveBaseDrops", fee.get("ReserveBase", 0))),
        "increment": drops(fee.get("ReserveIncrementDrops", fee.get("ReserveIncrement", 0))),
    }
    txs = []
    for t in sorted(child_bin["transactions"], key=lambda t: 0):
        pass
    by_hash = {}
    for t in child_bin["transactions"]:
        by_hash[t["hash"] if isinstance(t, dict) and "hash" in t else None] = t
    ordered = sorted(child["transactions"], key=lambda t: t["metaData"]["TransactionIndex"])
    for t in ordered:
        r = rpc("tx", transaction=t["hash"], binary=True)
        txs.append({"hash": t["hash"], "tx_hex": r["tx"], "metadata_hex": r["meta"]})

    manifest = {
        "kind": "manifest", "network_id": NETWORK_ID,
        "parent": {k: parent[k] for k in (
            "ledger_index", "total_coins", "ledger_hash", "parent_hash", "transaction_hash",
            "account_hash", "parent_close_time", "close_time", "close_time_resolution", "close_flags")},
        "child": {k: child[k] for k in (
            "ledger_index", "ledger_hash", "close_time", "close_time_resolution", "close_flags",
            "transaction_hash", "account_hash")},
        "fees": fees, "enabled_amendments": amendments, "transactions": txs,
    }
    with open(out_path, "w") as f:
        f.write(json.dumps(manifest) + "\n")
        for idx, data in sorted(rows.items()):
            f.write(json.dumps({"kind": "sle", "index": idx, "data": data}) + "\n")
    print(f"wrote {len(rows)} SLEs, {len(txs)} txs, {len(accounts)} accounts -> {out_path}")


if __name__ == "__main__":
    main()
