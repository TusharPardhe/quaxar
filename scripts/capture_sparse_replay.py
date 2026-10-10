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

    deleting = set()

    def collect_deleting(tx):
        if tx.get("TransactionType") == "AccountDelete":
            deleting.add(tx["Account"])
        for raw in tx.get("RawTransactions", []):
            collect_deleting(raw.get("RawTransaction", {}))

    for t in child["transactions"]:
        collect_deleting(t)
        collect_accounts(t)
        for n in t["metaData"]["AffectedNodes"]:
            kind = next(iter(n)); v = n[kind]
            if kind != "CreatedNode":
                want.add(v["LedgerIndex"])
            collect_accounts(v)

    def currency_bytes(code):
        if len(code) == 40:
            return bytes.fromhex(code)
        raw = bytearray(20)
        raw[12:15] = code.encode()
        return bytes(raw)

    def line_key(a, b, code):
        lo, hi = sorted([account_id(a), account_id(b)])
        return sha512h(b"\x00r" + lo + hi + currency_bytes(code))

    def collect_lines(tx):
        parties = {tx.get("Account"), tx.get("Destination"), tx.get("Owner")} - {None}
        for field in ("Amount", "SendMax", "DeliverMin", "TakerGets", "TakerPays",
                      "LimitAmount", "Amount2", "EPrice", "LPTokenOut", "LPTokenIn"):
            amt = tx.get(field)
            if isinstance(amt, dict) and "issuer" in amt and "currency" in amt:
                accounts.add(amt["issuer"])
                for party in parties:
                    if party != amt["issuer"]:
                        want.add(line_key(party, amt["issuer"], amt["currency"]))
        for raw in tx.get("RawTransactions", []):
            collect_lines(raw.get("RawTransaction", {}))

    for t in child["transactions"]:
        collect_lines(t)

    # Order books and AMM pools a transaction may cross: their offers are
    # read (quality, funding) even when the network leaves them untouched.
    def asset_of(amt):
        if isinstance(amt, dict):
            if "mpt_issuance_id" in amt:
                return None
            return {"currency": amt["currency"], "issuer": amt["issuer"]}
        if isinstance(amt, str):
            return {"currency": "XRP"}
        return None

    pairs = set()

    def add_pair(a, b):
        if a and b and a != b:
            pairs.add((json.dumps(a, sort_keys=True), json.dumps(b, sort_keys=True)))

    def collect_books(tx):
        if "TakerGets" in tx and "TakerPays" in tx:
            add_pair(asset_of(tx["TakerGets"]), asset_of(tx["TakerPays"]))
        if tx.get("TransactionType") == "Payment":
            src = asset_of(tx.get("SendMax", tx.get("Amount")))
            dst = asset_of(tx.get("Amount"))
            add_pair(src, dst)
            for path in tx.get("Paths", []):
                cur = src
                for step in path:
                    nxt = None
                    if "currency" in step:
                        nxt = {"currency": step["currency"]} if step["currency"] == "XRP" else {
                            "currency": step["currency"], "issuer": step.get("issuer", (cur or {}).get("issuer"))}
                    if nxt:
                        add_pair(cur, nxt)
                        cur = nxt
                add_pair(cur, dst)
        for raw in tx.get("RawTransactions", []):
            collect_books(raw.get("RawTransaction", {}))

    for t in child["transactions"]:
        collect_books(t)

    for a_json, b_json in list(pairs):
        a, b = json.loads(a_json), json.loads(b_json)
        for gets, pays in ((a, b), (b, a)):
            try:
                book = rpc("book_offers", taker_gets=gets, taker_pays=pays,
                           ledger_index=parent_seq, limit=100)
            except RuntimeError:
                book = {}
            for offer in book.get("offers", []):
                want.add(offer["index"])
                want.add(offer["BookDirectory"])
                accounts.add(offer["Account"])
                for side in (offer.get("TakerGets"), offer.get("TakerPays")):
                    if isinstance(side, dict) and "issuer" in side:
                        want.add(line_key(offer["Account"], side["issuer"], side["currency"]))
        try:
            amm = rpc("amm_info", asset=a, asset2=b, ledger_index=parent_seq)["amm"]
            accounts.add(amm["account"])
            for side in (a, b):
                if "issuer" in side:
                    want.add(line_key(amm["account"], side["issuer"], side["currency"]))
        except (RuntimeError, KeyError):
            pass

    for a in accounts:
        want.add(account_key(a))
        want.add(sha512h(b"\x00S" + account_id(a) + (0).to_bytes(4, "big")))  # SignerList
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
        # The issuer AccountID is bytes 4..24 of the NFTokenID.
        want.add(sha512h(b"\x00a" + bytes.fromhex(nid[8:48])))
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
        if j.get("LedgerEntryType") == "AccountRoot" and "AMMID" in j:
            pending.append(j["AMMID"])
        if j.get("LedgerEntryType") == "DirectoryNode":
            root = j.get("RootIndex", idx)
            for page_field in ("IndexNext", "IndexPrevious"):
                p = int(j.get(page_field, "0"), 16)
                if p:
                    pending.append(dir_page_key(root, p))
            # owner directories: include every owned entry (reads during deletes)
            # Every owned object is only read wholesale by AccountDelete;
            # NFToken offer directories are walked by NFT transactions.
            if "NFTokenID" in j or j.get("Owner") in deleting:
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
