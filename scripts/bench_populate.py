#!/usr/bin/env python3
"""Populate a standalone XRPL server with the benchmark ledger and record
request bodies for the matched quaxar-vs-rippled benchmark.

Usage: bench_populate.py <target-url> [<signer-url>] [--network]

--network: the target is not standalone (no ledger_accept); each step waits
until the target's validated ledger has advanced past the open ledger the
step was submitted into.

Transactions are signed offline by the signer (rippled) with sequence
numbers read from the target, then submitted to the target with
ledger_accept closes, so every server ends with equivalent state:
two funded accounts, a USD trust line, 200 offers in one order book, and
200 tickets.
"""

import json
import sys
import urllib.request

OFFERS = 200
TICKETS = 200


def rpc(url, method, params=None):
    body = json.dumps({"method": method, "params": [params or {}]}).encode()
    req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.load(resp)["result"]


def wallet(url, passphrase):
    r = rpc(url, "wallet_propose", {"passphrase": passphrase})
    return r["account_id"], r["master_seed"]


def sign(url, secret, tx):
    r = rpc(url, "sign", {"tx_json": tx, "secret": secret, "offline": True})
    assert "tx_blob" in r, r
    return r["tx_blob"]


def next_seq(url, account):
    r = rpc(url, "account_info", {"account": account, "ledger_index": "current"})
    return r["account_data"]["Sequence"]


FEE = "1000000"  # 1 XRP: above open-ledger fee escalation for 200 txs/ledger


def make_closer(target, network):
    import time

    def close():
        if not network:
            rpc(target, "ledger_accept")
            return
        submitted_into = rpc(target, "ledger_current")["ledger_current_index"]
        for _ in range(240):
            info = rpc(target, "server_info")["info"]
            if (info.get("validated_ledger") or {}).get("seq", 0) >= submitted_into:
                return
            time.sleep(0.5)
        raise SystemExit("target did not validate a ledger in 120 s")

    return close


def populate(target, signer, network=False):
    """Build the benchmark ledger on `target`, signing offline on `signer`
    (rippled) with sequence numbers read from the target."""
    genesis, genesis_secret = wallet(signer, "masterpassphrase")
    gateway, gateway_secret = wallet(signer, "bench-gateway")
    holder, holder_secret = wallet(signer, "bench-holder")
    applied = []
    close = make_closer(target, network)
    if network:
        import time
        for _ in range(240):
            info = rpc(target, "server_info")["info"]
            if (info.get("validated_ledger") or {}).get("seq"):
                break
            time.sleep(0.5)

    def submit(secret, tx):
        blob = sign(signer, secret, tx)
        r = rpc(target, "submit", {"tx_blob": blob})
        assert r.get("engine_result") in ("tesSUCCESS", "terQUEUED"), r
        applied.append(r["tx_json"]["hash"])

    seq = next_seq(target, genesis)
    for dest in (gateway, holder):
        submit(genesis_secret, {
            "TransactionType": "Payment", "Account": genesis, "Destination": dest,
            "Amount": "10000000000", "Fee": FEE, "Sequence": seq})
        seq += 1
    close()

    hseq = next_seq(target, holder)
    submit(holder_secret, {
        "TransactionType": "TrustSet", "Account": holder, "Fee": FEE, "Sequence": hseq,
        "LimitAmount": {"currency": "USD", "issuer": gateway, "value": "1000000"}})
    hseq += 1
    close()
    gseq = next_seq(target, gateway)
    submit(gateway_secret, {
        "TransactionType": "Payment", "Account": gateway, "Destination": holder, "Fee": FEE,
        "Sequence": gseq, "Amount": {"currency": "USD", "issuer": gateway, "value": "5000"}})
    gseq += 1
    close()

    sample = None
    for i in range(OFFERS):
        submit(gateway_secret, {
            "TransactionType": "OfferCreate", "Account": gateway, "Fee": FEE,
            "Sequence": gseq,
            "TakerPays": str(1_000_000 + 1_000 * i),
            "TakerGets": {"currency": "USD", "issuer": gateway, "value": "1.234567890123456"}})
        gseq += 1
        sample = sample or applied[-1]
        if (i + 1) % (20 if network else 8) == 0:
            close()
    close()

    submit(holder_secret, {
        "TransactionType": "TicketCreate", "Account": holder, "Fee": FEE,
        "Sequence": hseq, "TicketCount": TICKETS})
    close()
    close()
    print(json.dumps({"applied": len(applied), "gateway": gateway, "holder": holder,
                      "sample_tx": sample}))


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    populate(args[0], args[1] if len(args) > 1 else args[0], "--network" in sys.argv)
