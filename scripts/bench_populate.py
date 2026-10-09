#!/usr/bin/env python3
"""Populate a standalone XRPL server with the benchmark ledger and record
request bodies for the matched quaxar-vs-rippled benchmark.

Usage: bench_populate.py <target-url> [<signer-url>]

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


def populate(target, signer):
    """Build the benchmark ledger on `target`, signing offline on `signer`
    (rippled) with sequence numbers read from the target."""
    genesis, genesis_secret = wallet(signer, "masterpassphrase")
    gateway, gateway_secret = wallet(signer, "bench-gateway")
    holder, holder_secret = wallet(signer, "bench-holder")
    applied = []

    def submit(secret, tx):
        blob = sign(signer, secret, tx)
        r = rpc(target, "submit", {"tx_blob": blob})
        assert r.get("engine_result") in ("tesSUCCESS", "terQUEUED"), r
        applied.append(r["tx_json"]["hash"])

    seq = next_seq(target, genesis)
    for dest in (gateway, holder):
        submit(genesis_secret, {
            "TransactionType": "Payment", "Account": genesis, "Destination": dest,
            "Amount": "10000000000", "Fee": "10", "Sequence": seq})
        seq += 1
    rpc(target, "ledger_accept")

    hseq = next_seq(target, holder)
    submit(holder_secret, {
        "TransactionType": "TrustSet", "Account": holder, "Fee": "10", "Sequence": hseq,
        "LimitAmount": {"currency": "USD", "issuer": gateway, "value": "1000000"}})
    hseq += 1
    rpc(target, "ledger_accept")
    gseq = next_seq(target, gateway)
    submit(gateway_secret, {
        "TransactionType": "Payment", "Account": gateway, "Destination": holder, "Fee": "10",
        "Sequence": gseq, "Amount": {"currency": "USD", "issuer": gateway, "value": "5000"}})
    gseq += 1
    rpc(target, "ledger_accept")

    sample = None
    for i in range(OFFERS):
        submit(gateway_secret, {
            "TransactionType": "OfferCreate", "Account": gateway, "Fee": "10",
            "Sequence": gseq,
            "TakerPays": str(1_000_000 + 1_000 * i),
            "TakerGets": {"currency": "USD", "issuer": gateway, "value": "1.234567890123456"}})
        gseq += 1
        sample = sample or applied[-1]
        if (i + 1) % 8 == 0:
            rpc(target, "ledger_accept")
    rpc(target, "ledger_accept")

    submit(holder_secret, {
        "TransactionType": "TicketCreate", "Account": holder, "Fee": "10",
        "Sequence": hseq, "TicketCount": TICKETS})
    rpc(target, "ledger_accept")
    rpc(target, "ledger_accept")
    print(json.dumps({"applied": len(applied), "gateway": gateway, "holder": holder,
                      "sample_tx": sample}))


if __name__ == "__main__":
    populate(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else sys.argv[1])
