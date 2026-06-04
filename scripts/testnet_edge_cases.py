#!/usr/bin/env python3
"""
Real testnet integration tests for xrpld.
Submits actual transactions to the XRP Testnet with complex flag combinations.

Usage:
    python3 scripts/testnet_edge_cases.py [--node http://18.134.130.94:5005]

Default uses public testnet: https://s.altnet.rippletest.net:51234
"""

import json, time, sys, hashlib, secrets, struct
from urllib.request import urlopen, Request
from urllib.error import URLError

# --- Config ---
NODE_URL = "https://s.altnet.rippletest.net:51234"
FAUCET_URL = "https://faucet.altnet.rippletest.net/accounts"

if "--node" in sys.argv:
    NODE_URL = sys.argv[sys.argv.index("--node") + 1]

# --- Helpers ---
def rpc(method, params=None):
    payload = {"method": method, "params": [params or {}]}
    req = Request(NODE_URL, json.dumps(payload).encode(), {"Content-Type": "application/json"})
    resp = json.loads(urlopen(req, timeout=30).read())
    return resp.get("result", resp)

def fund_account():
    """Fund a new account from the testnet faucet."""
    req = Request(FAUCET_URL, method="POST")
    req.add_header("Content-Type", "application/json")
    resp = json.loads(urlopen(req, timeout=30).read())
    address = resp["account"]["classicAddress"]
    secret = resp["seed"]
    print(f"  Funded: {address}")
    time.sleep(4)  # Wait for ledger close
    return {"address": address, "secret": secret}

def submit_tx(tx, secret):
    """Sign and submit a transaction."""
    tx.setdefault("Fee", "12")
    sign_resp = rpc("sign", {"tx_json": tx, "secret": secret})
    if sign_resp.get("status") != "success":
        return sign_resp
    tx_blob = sign_resp["tx_blob"]
    result = rpc("submit", {"tx_blob": tx_blob})
    return result

def wait_validated(tx_hash, timeout=15):
    """Wait for a transaction to be validated."""
    end = time.time() + timeout
    while time.time() < end:
        resp = rpc("tx", {"transaction": tx_hash})
        if resp.get("validated"):
            return resp
        time.sleep(1)
    return None

def submit_and_wait(tx, secret, label=""):
    """Submit and wait for validation. Returns (engine_result, full_response)."""
    result = submit_tx(tx, secret)
    engine_result = result.get("engine_result", "UNKNOWN")
    tx_hash = result.get("tx_json", {}).get("hash", "")
    
    if engine_result.startswith("tes"):
        validated = wait_validated(tx_hash)
        if validated:
            engine_result = validated.get("meta", {}).get("TransactionResult", engine_result)
    
    status = "✓" if engine_result == "tesSUCCESS" else "✗" if "tec" in engine_result or "tem" in engine_result or "tef" in engine_result else "?"
    print(f"  {status} {label}: {engine_result}")
    return engine_result, result

def get_account_info(address):
    return rpc("account_info", {"account": address, "ledger_index": "validated"})

# --- Test Suites ---

def test_account_settings():
    """Test complex AccountSet flag combinations."""
    print("\n═══ ACCOUNT SETTINGS ═══")
    acct = fund_account()
    addr, secret = acct["address"], acct["secret"]
    
    # 1. Set RequireDestTag (flag 1)
    submit_and_wait({"TransactionType": "AccountSet", "Account": addr, "SetFlag": 1}, secret,
                    "Set RequireDestTag (asfRequireDestTag=1)")
    
    # 2. Set RequireAuth (flag 2) 
    submit_and_wait({"TransactionType": "AccountSet", "Account": addr, "SetFlag": 2}, secret,
                    "Set RequireAuth (asfRequireAuth=2)")
    
    # 3. Set DisallowXRP (flag 3)
    submit_and_wait({"TransactionType": "AccountSet", "Account": addr, "SetFlag": 3}, secret,
                    "Set DisallowXRP (asfDisallowXRP=3)")
    
    # 4. Set TransferRate (1.5x = 1500000000)
    submit_and_wait({"TransactionType": "AccountSet", "Account": addr, "TransferRate": 1500000000}, secret,
                    "Set TransferRate 1.5x")
    
    # 5. Set TickSize = 5
    submit_and_wait({"TransactionType": "AccountSet", "Account": addr, "TickSize": 5}, secret,
                    "Set TickSize=5")
    
    # 6. Set Domain
    submit_and_wait({"TransactionType": "AccountSet", "Account": addr, "Domain": "746573742E636F6D"}, secret,
                    "Set Domain 'test.com'")
    
    # 7. Clear Domain (empty)
    submit_and_wait({"TransactionType": "AccountSet", "Account": addr, "Domain": ""}, secret,
                    "Clear Domain (empty)")
    
    # 8. Set NoFreeze (irreversible, flag 6)
    submit_and_wait({"TransactionType": "AccountSet", "Account": addr, "SetFlag": 6}, secret,
                    "Set NoFreeze (asfNoFreeze=6) - IRREVERSIBLE")
    
    # 9. Try to clear NoFreeze (should fail)
    submit_and_wait({"TransactionType": "AccountSet", "Account": addr, "ClearFlag": 6}, secret,
                    "Clear NoFreeze (should fail - tecOWNERS or noop)")
    
    # 10. Set AllowTrustLineClawback (flag 16)
    acct2 = fund_account()
    submit_and_wait({"TransactionType": "AccountSet", "Account": acct2["address"], "SetFlag": 16}, acct2["secret"],
                    "Set AllowTrustLineClawback (asfAllowTrustLineClawback=16)")
    
    # 11. Conflicting set and clear same flag
    submit_and_wait({"TransactionType": "AccountSet", "Account": addr, "SetFlag": 1, "ClearFlag": 1}, secret,
                    "Conflicting Set+Clear same flag (should fail temINVALID_FLAG)")

def test_trust_lines_and_freeze():
    """Test TrustSet with freeze, deep freeze, NoRipple flags."""
    print("\n═══ TRUST LINES & FREEZE ═══")
    issuer = fund_account()
    holder = fund_account()
    
    # 1. Create trust line
    submit_and_wait({
        "TransactionType": "TrustSet", "Account": holder["address"],
        "LimitAmount": {"currency": "USD", "issuer": issuer["address"], "value": "1000"}
    }, holder["secret"], "Create USD trust line (limit 1000)")
    
    # 2. Set NoRipple on trust line
    submit_and_wait({
        "TransactionType": "TrustSet", "Account": holder["address"],
        "LimitAmount": {"currency": "USD", "issuer": issuer["address"], "value": "1000"},
        "Flags": 131072  # tfSetNoRipple
    }, holder["secret"], "Set NoRipple on trust line")
    
    # 3. Issuer freezes the trust line
    submit_and_wait({
        "TransactionType": "TrustSet", "Account": issuer["address"],
        "LimitAmount": {"currency": "USD", "issuer": holder["address"], "value": "0"},
        "Flags": 1048576  # tfSetFreeze
    }, issuer["secret"], "Issuer freezes trust line")
    
    # 4. Payment on frozen line (should fail)
    submit_and_wait({
        "TransactionType": "Payment", "Account": issuer["address"],
        "Destination": holder["address"],
        "Amount": {"currency": "USD", "issuer": issuer["address"], "value": "100"}
    }, issuer["secret"], "Payment on frozen trust line (should fail tecPATH_DRY)")
    
    # 5. Issuer unfreezes
    submit_and_wait({
        "TransactionType": "TrustSet", "Account": issuer["address"],
        "LimitAmount": {"currency": "USD", "issuer": holder["address"], "value": "0"},
        "Flags": 2097152  # tfClearFreeze
    }, issuer["secret"], "Issuer unfreezes trust line")
    
    # 6. GlobalFreeze
    submit_and_wait({"TransactionType": "AccountSet", "Account": issuer["address"], "SetFlag": 7},
                    issuer["secret"], "Set GlobalFreeze (asfGlobalFreeze=7)")
    
    # 7. Payment during global freeze (should fail)
    submit_and_wait({
        "TransactionType": "Payment", "Account": issuer["address"],
        "Destination": holder["address"],
        "Amount": {"currency": "USD", "issuer": issuer["address"], "value": "50"}
    }, issuer["secret"], "Payment during GlobalFreeze (should fail)")

def test_offers_and_dex():
    """Test OfferCreate with complex flags."""
    print("\n═══ OFFERS & DEX ═══")
    alice = fund_account()
    bob = fund_account()
    
    # Setup: create trust lines and fund IOUs
    submit_and_wait({
        "TransactionType": "TrustSet", "Account": bob["address"],
        "LimitAmount": {"currency": "USD", "issuer": alice["address"], "value": "10000"}
    }, bob["secret"], "Bob trusts Alice for USD")
    
    submit_and_wait({
        "TransactionType": "Payment", "Account": alice["address"],
        "Destination": bob["address"],
        "Amount": {"currency": "USD", "issuer": alice["address"], "value": "500"}
    }, alice["secret"], "Alice sends 500 USD to Bob")
    
    # 1. Place sell offer: Bob sells 100 USD for 50 XRP
    submit_and_wait({
        "TransactionType": "OfferCreate", "Account": bob["address"],
        "TakerGets": {"currency": "USD", "issuer": alice["address"], "value": "100"},
        "TakerPays": "50000000"
    }, bob["secret"], "Bob places sell offer: 100 USD for 50 XRP")
    
    # 2. FillOrKill offer that can't fill (should fail)
    submit_and_wait({
        "TransactionType": "OfferCreate", "Account": alice["address"],
        "TakerGets": "500000000",  # 500 XRP (more than Bob has)
        "TakerPays": {"currency": "USD", "issuer": alice["address"], "value": "1000"},
        "Flags": 262144  # tfFillOrKill
    }, alice["secret"], "FillOrKill that can't fill (should fail tecKILLED)")
    
    # 3. ImmediateOrCancel partial fill
    submit_and_wait({
        "TransactionType": "OfferCreate", "Account": alice["address"],
        "TakerGets": "25000000",  # 25 XRP
        "TakerPays": {"currency": "USD", "issuer": alice["address"], "value": "50"},
        "Flags": 131072  # tfImmediateOrCancel
    }, alice["secret"], "ImmediateOrCancel (partial cross)")
    
    # 4. Offer with Expiration in the past (should fail)
    submit_and_wait({
        "TransactionType": "OfferCreate", "Account": alice["address"],
        "TakerGets": "10000000",
        "TakerPays": {"currency": "USD", "issuer": alice["address"], "value": "10"},
        "Expiration": 1  # epoch + 1 second (far in past)
    }, alice["secret"], "Offer with past expiration (should fail tecEXPIRED)")

def test_escrow():
    """Test Escrow create/finish/cancel with time conditions."""
    print("\n═══ ESCROW ═══")
    alice = fund_account()
    bob = fund_account()
    
    # Get current ledger close time
    info = rpc("server_info")
    close_time = info.get("info", {}).get("validated_ledger", {}).get("close_time", 0)
    ripple_epoch_offset = 946684800
    
    # 1. Create escrow with FinishAfter (5 seconds from now)
    finish_after = close_time + 5
    cancel_after = close_time + 3600  # 1 hour
    
    submit_and_wait({
        "TransactionType": "EscrowCreate", "Account": alice["address"],
        "Destination": bob["address"],
        "Amount": "10000000",  # 10 XRP
        "FinishAfter": finish_after,
        "CancelAfter": cancel_after
    }, alice["secret"], f"Create escrow (FinishAfter=+5s, CancelAfter=+1hr)")
    
    # 2. Try to finish too early (should fail)
    submit_and_wait({
        "TransactionType": "EscrowFinish", "Account": bob["address"],
        "Owner": alice["address"], "OfferSequence": 1
    }, bob["secret"], "Finish escrow too early (should fail tecNO_PERMISSION)")
    
    # 3. Wait and finish
    print("  ⏳ Waiting 8 seconds for FinishAfter...")
    time.sleep(8)
    
    submit_and_wait({
        "TransactionType": "EscrowFinish", "Account": bob["address"],
        "Owner": alice["address"], "OfferSequence": 1
    }, bob["secret"], "Finish escrow after FinishAfter")
    
    # 4. Create escrow for cancel test
    submit_and_wait({
        "TransactionType": "EscrowCreate", "Account": alice["address"],
        "Destination": bob["address"],
        "Amount": "5000000",
        "CancelAfter": close_time + 5  # Cancellable in 5s
    }, alice["secret"], "Create escrow for cancel (CancelAfter=+5s)")
    
    # 5. Cancel before time (should fail)
    submit_and_wait({
        "TransactionType": "EscrowCancel", "Account": alice["address"],
        "Owner": alice["address"], "OfferSequence": 2
    }, alice["secret"], "Cancel before CancelAfter (should fail tecNO_PERMISSION)")

def test_checks():
    """Test Check create/cash/cancel."""
    print("\n═══ CHECKS ═══")
    alice = fund_account()
    bob = fund_account()
    
    # 1. Create check
    submit_and_wait({
        "TransactionType": "CheckCreate", "Account": alice["address"],
        "Destination": bob["address"],
        "SendMax": "50000000"  # 50 XRP
    }, alice["secret"], "Create check for 50 XRP")
    
    # Get the check ID from account_objects
    time.sleep(4)
    objects = rpc("account_objects", {"account": alice["address"], "type": "check"})
    checks = objects.get("account_objects", [])
    
    if checks:
        check_id = checks[0]["index"]
        
        # 2. Bob cashes the check
        submit_and_wait({
            "TransactionType": "CheckCash", "Account": bob["address"],
            "CheckID": check_id, "Amount": "50000000"
        }, bob["secret"], "Bob cashes check for 50 XRP")
    
    # 3. Create another check and cancel it
    submit_and_wait({
        "TransactionType": "CheckCreate", "Account": alice["address"],
        "Destination": bob["address"],
        "SendMax": "25000000"
    }, alice["secret"], "Create check for cancel test")
    
    time.sleep(4)
    objects = rpc("account_objects", {"account": alice["address"], "type": "check"})
    checks = objects.get("account_objects", [])
    
    if checks:
        check_id = checks[0]["index"]
        # 4. Third party cancel (should fail for unexpired)
        charlie = fund_account()
        submit_and_wait({
            "TransactionType": "CheckCancel", "Account": charlie["address"],
            "CheckID": check_id
        }, charlie["secret"], "Third party cancel unexpired check (should fail tecNO_PERMISSION)")
        
        # 5. Owner cancel (should succeed)
        submit_and_wait({
            "TransactionType": "CheckCancel", "Account": alice["address"],
            "CheckID": check_id
        }, alice["secret"], "Owner cancels own check")

def test_payment_channels():
    """Test PaymentChannel create/fund/claim."""
    print("\n═══ PAYMENT CHANNELS ═══")
    alice = fund_account()
    bob = fund_account()
    
    # 1. Create channel
    submit_and_wait({
        "TransactionType": "PaymentChannelCreate", "Account": alice["address"],
        "Destination": bob["address"],
        "Amount": "100000000",  # 100 XRP
        "SettleDelay": 86400,  # 1 day
        "PublicKey": "023693F15967AE357D0327974AD46FE3C127113B1110D6044FD41E723689F81CC6"
    }, alice["secret"], "Create payment channel (100 XRP, 1 day settle)")
    
    time.sleep(4)
    
    # Get channel ID
    objects = rpc("account_objects", {"account": alice["address"], "type": "payment_channel"})
    channels = objects.get("account_objects", [])
    
    if channels:
        channel_id = channels[0]["index"]
        
        # 2. Fund channel (add 50 XRP)
        submit_and_wait({
            "TransactionType": "PaymentChannelFund", "Account": alice["address"],
            "Channel": channel_id, "Amount": "50000000"
        }, alice["secret"], "Fund channel +50 XRP")
        
        # 3. Non-owner fund (should fail)
        submit_and_wait({
            "TransactionType": "PaymentChannelFund", "Account": bob["address"],
            "Channel": channel_id, "Amount": "10000000"
        }, bob["secret"], "Non-owner fund (should fail tecNO_PERMISSION)")
        
        # 4. Close channel (tfClose)
        submit_and_wait({
            "TransactionType": "PaymentChannelClaim", "Account": alice["address"],
            "Channel": channel_id, "Flags": 131072  # tfClose
        }, alice["secret"], "Close channel (tfClose)")

def test_nftoken():
    """Test NFToken mint/burn/offer/modify."""
    print("\n═══ NFTOKENS ═══")
    alice = fund_account()
    bob = fund_account()
    
    # 1. Mint NFT (transferable + mutable)
    submit_and_wait({
        "TransactionType": "NFTokenMint", "Account": alice["address"],
        "NFTokenTaxon": 0,
        "Flags": 8 + 16,  # tfTransferable + tfMutable (if DynamicNFT enabled)
        "URI": "68747470733A2F2F6578616D706C652E636F6D2F6E6674"  # https://example.com/nft
    }, alice["secret"], "Mint NFT (transferable+mutable)")
    
    time.sleep(4)
    
    # Get token ID
    nfts = rpc("account_nfts", {"account": alice["address"]})
    tokens = nfts.get("account_nfts", [])
    
    if tokens:
        token_id = tokens[0]["NFTokenID"]
        
        # 2. Create sell offer
        submit_and_wait({
            "TransactionType": "NFTokenCreateOffer", "Account": alice["address"],
            "NFTokenID": token_id,
            "Amount": "10000000",  # 10 XRP
            "Flags": 1  # tfSellNFToken
        }, alice["secret"], "Create sell offer (10 XRP)")
        
        time.sleep(4)
        
        # Get offer ID
        offers = rpc("nft_sell_offers", {"nft_id": token_id})
        sell_offers = offers.get("offers", [])
        
        if sell_offers:
            offer_id = sell_offers[0]["nft_offer_index"]
            
            # 3. Bob accepts offer
            submit_and_wait({
                "TransactionType": "NFTokenAcceptOffer", "Account": bob["address"],
                "NFTokenSellOffer": offer_id
            }, bob["secret"], "Bob accepts sell offer")
        
        # 4. Bob burns the NFT
        submit_and_wait({
            "TransactionType": "NFTokenBurn", "Account": bob["address"],
            "NFTokenID": token_id
        }, bob["secret"], "Bob burns NFT")
    
    # 5. Mint with TransferFee (5%)
    submit_and_wait({
        "TransactionType": "NFTokenMint", "Account": alice["address"],
        "NFTokenTaxon": 1,
        "TransferFee": 5000,  # 5%
        "Flags": 8  # tfTransferable
    }, alice["secret"], "Mint NFT with 5% TransferFee")

def test_deposit_preauth():
    """Test DepositPreauth interactions."""
    print("\n═══ DEPOSIT PREAUTH ═══")
    alice = fund_account()
    bob = fund_account()
    
    # 1. Alice enables DepositAuth
    submit_and_wait({"TransactionType": "AccountSet", "Account": alice["address"], "SetFlag": 9},
                    alice["secret"], "Alice sets DepositAuth (asfDepositAuth=9)")
    
    # 2. Bob tries to pay Alice (should fail)
    submit_and_wait({
        "TransactionType": "Payment", "Account": bob["address"],
        "Destination": alice["address"], "Amount": "10000000"
    }, bob["secret"], "Bob pays Alice without preauth (should fail tecNO_PERMISSION)")
    
    # 3. Alice preauthorizes Bob
    submit_and_wait({
        "TransactionType": "DepositPreauth", "Account": alice["address"],
        "Authorize": bob["address"]
    }, alice["secret"], "Alice preauthorizes Bob")
    
    # 4. Bob pays Alice (should succeed now)
    submit_and_wait({
        "TransactionType": "Payment", "Account": bob["address"],
        "Destination": alice["address"], "Amount": "10000000"
    }, bob["secret"], "Bob pays Alice with preauth (should succeed)")
    
    # 5. Remove preauth
    submit_and_wait({
        "TransactionType": "DepositPreauth", "Account": alice["address"],
        "Unauthorize": bob["address"]
    }, alice["secret"], "Alice removes Bob's preauth")

def test_multisign():
    """Test multi-signing with SignerListSet."""
    print("\n═══ MULTISIGN ═══")
    master = fund_account()
    signer1 = fund_account()
    signer2 = fund_account()
    
    # 1. Set signer list (quorum 2, weight 1 each)
    submit_and_wait({
        "TransactionType": "SignerListSet", "Account": master["address"],
        "SignerQuorum": 2,
        "SignerEntries": [
            {"SignerEntry": {"Account": signer1["address"], "SignerWeight": 1}},
            {"SignerEntry": {"Account": signer2["address"], "SignerWeight": 1}}
        ]
    }, master["secret"], "Set signer list (quorum=2, 2 signers weight=1)")
    
    # 2. Disable master key
    submit_and_wait({"TransactionType": "AccountSet", "Account": master["address"], "SetFlag": 4},
                    master["secret"], "Disable master key (asfDisableMaster=4)")
    
    # 3. Try single-sign with master (should fail)
    submit_and_wait({
        "TransactionType": "Payment", "Account": master["address"],
        "Destination": signer1["address"], "Amount": "1000000"
    }, master["secret"], "Payment with disabled master (should fail tefMASTER_DISABLED)")

# --- Main ---
if __name__ == "__main__":
    print("╔══════════════════════════════════════════════╗")
    print("║  XRP Testnet Edge Case Integration Tests    ║")
    print(f"║  Node: {NODE_URL:<38}║")
    print("╚══════════════════════════════════════════════╝")
    
    try:
        info = rpc("server_info")
        state = info.get("info", {}).get("server_state", "?")
        print(f"\nServer state: {state}")
        print(f"Network ID: {info.get('info', {}).get('network_id', '?')}")
    except Exception as e:
        print(f"\n⚠ Cannot reach node: {e}")
        print("  Using public testnet endpoint instead.")
        NODE_URL = "https://s.altnet.rippletest.net:51234"
    
    tests = [
        test_account_settings,
        test_trust_lines_and_freeze,
        test_offers_and_dex,
        test_escrow,
        test_checks,
        test_payment_channels,
        test_nftoken,
        test_deposit_preauth,
        test_multisign,
    ]
    
    passed = 0
    failed = 0
    
    for test_fn in tests:
        try:
            test_fn()
        except Exception as e:
            print(f"  ⚠ Test suite error: {e}")
            failed += 1
    
    print(f"\n{'═'*50}")
    print(f"Done. Run against: {NODE_URL}")
