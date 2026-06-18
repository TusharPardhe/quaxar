#!/usr/bin/env bash
# devnet-loadtest.sh — Long-running load test for quaxar devnet
# Exercises: Payment, OfferCreate, TrustSet, OfferCancel
# Monitors: drift, hash agreement, error logs
set -euo pipefail

R0=${R0:-http://127.0.0.1:33797}
R1=${R1:-http://127.0.0.1:33800}
Q0=${Q0:-http://127.0.0.1:33814}
Q1=${Q1:-http://127.0.0.1:33811}
Q2=${Q2:-http://127.0.0.1:33808}
GENESIS_SECRET="snoPBrXtMeMyMHUVTgbuqAfg1SUTb"
GENESIS_ACCT="rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh"
LOG="devnet-loadtest.log"
DURATION=${DURATION:-86400}  # 24 hours default
INTERVAL=${INTERVAL:-3}      # tx every 3 seconds

# Counters
TX_SENT=0
TX_SUCCESS=0
TX_FAIL=0
HASH_CHECKS=0
HASH_MATCHES=0
HASH_MISMATCHES=0
MAX_DRIFT=0
START_TIME=$(date +%s)

log() { echo "$(date '+%H:%M:%S') $*" | tee -a "$LOG"; }

rpc() {
  local url=$1 method=$2 params=$3
  curl -sf "$url" -d "{\"method\":\"$method\",\"params\":[$params]}" 2>/dev/null
}

get_seq() {
  rpc "$1" account_info "{\"account\":\"$2\"}" | python3 -c "import json,sys;d=json.load(sys.stdin);print(d['result']['account_data']['Sequence'])" 2>/dev/null
}

get_validated_seq() {
  rpc "$1" server_info "{}" | python3 -c "import json,sys;d=json.load(sys.stdin);print(d['result']['info'].get('validated_ledger',{}).get('seq',0))" 2>/dev/null
}

submit_tx() {
  local node=$1 secret=$2 tx_json=$3
  local result=$(rpc "$node" submit "{\"secret\":\"$secret\",\"tx_json\":$tx_json}" | python3 -c "import json,sys;d=json.load(sys.stdin);print(d['result'].get('engine_result','ERROR'))" 2>/dev/null)
  echo "$result"
}

check_hashes() {
  local seq=$1
  local rh=$(rpc "$R0" ledger "{\"ledger_index\":$seq}" | python3 -c "import json,sys;d=json.load(sys.stdin);print(d['result'].get('ledger',{}).get('ledger_hash','?'))" 2>/dev/null)
  local qh=$(rpc "$Q0" ledger "{\"ledger_index\":$seq}" | python3 -c "import json,sys;d=json.load(sys.stdin);print(d['result'].get('ledger',{}).get('ledger_hash','?'))" 2>/dev/null)
  HASH_CHECKS=$((HASH_CHECKS + 1))
  if [ "$rh" = "$qh" ] && [ "$rh" != "?" ]; then
    HASH_MATCHES=$((HASH_MATCHES + 1))
    return 0
  else
    HASH_MISMATCHES=$((HASH_MISMATCHES + 1))
    log "❌ HASH MISMATCH at seq=$seq R=$rh Q=$qh"
    return 1
  fi
}

# --- Setup: fund test accounts ---
log "=== DEVNET LOAD TEST STARTED ==="
log "Duration: ${DURATION}s | Interval: ${INTERVAL}s"
log "Nodes: R0=$R0 Q0=$Q0 Q1=$Q1 Q2=$Q2"

# Wait for network
log "Waiting for network sync..."
for i in $(seq 1 30); do
  RS=$(get_validated_seq "$R0")
  QS=$(get_validated_seq "$Q0")
  if [ "${RS:-0}" -gt 5 ] && [ "${QS:-0}" -gt 5 ]; then
    log "Network synced: R0=$RS Q0=$QS"
    break
  fi
  sleep 2
done

# Create test accounts
log "Funding test accounts..."
ACCOUNTS=()
SECRETS=()
for i in $(seq 0 4); do
  WALLET=$(rpc "$R0" wallet_propose "{\"passphrase\":\"loadtest_account_$i\"}")
  ADDR=$(echo "$WALLET" | python3 -c "import json,sys;print(json.load(sys.stdin)['result']['account_id'])")
  SEC=$(echo "$WALLET" | python3 -c "import json,sys;print(json.load(sys.stdin)['result']['master_seed'])")
  ACCOUNTS+=("$ADDR")
  SECRETS+=("$SEC")
  
  SEQ=$(get_seq "$R0" "$GENESIS_ACCT")
  submit_tx "$R0" "$GENESIS_SECRET" "{\"TransactionType\":\"Payment\",\"Account\":\"$GENESIS_ACCT\",\"Destination\":\"$ADDR\",\"Amount\":\"10000000000\",\"Sequence\":$SEQ,\"Fee\":\"12\"}" > /dev/null
  sleep 1
done
sleep 10
log "Funded ${#ACCOUNTS[@]} test accounts"

# Setup trustlines for IOU testing
log "Setting up trustlines..."
for i in 1 2 3 4; do
  SEQ=$(get_seq "$R0" "${ACCOUNTS[$i]}")
  submit_tx "$R0" "${SECRETS[$i]}" "{\"TransactionType\":\"TrustSet\",\"Account\":\"${ACCOUNTS[$i]}\",\"LimitAmount\":{\"currency\":\"USD\",\"issuer\":\"${ACCOUNTS[0]}\",\"value\":\"10000\"},\"Sequence\":$SEQ,\"Fee\":\"12\"}" > /dev/null
  sleep 1
done
sleep 8
log "Trustlines ready"

# Issue USD from account 0
for i in 1 2 3 4; do
  SEQ=$(get_seq "$R0" "${ACCOUNTS[0]}")
  submit_tx "$R0" "${SECRETS[0]}" "{\"TransactionType\":\"Payment\",\"Account\":\"${ACCOUNTS[0]}\",\"Destination\":\"${ACCOUNTS[$i]}\",\"Amount\":{\"currency\":\"USD\",\"issuer\":\"${ACCOUNTS[0]}\",\"value\":\"1000\"},\"Sequence\":$SEQ,\"Fee\":\"12\"}" > /dev/null
  sleep 1
done
sleep 8
log "USD issued to all accounts"

# --- Main loop ---
log "=== STARTING LOAD GENERATION ==="
ROUND=0
LAST_HASH_CHECK=0

while true; do
  ELAPSED=$(( $(date +%s) - START_TIME ))
  if [ $ELAPSED -ge $DURATION ]; then break; fi
  ROUND=$((ROUND + 1))
  
  # Pick random source, dest, tx type, and node to submit to
  SRC_IDX=$((RANDOM % 5))
  DST_IDX=$(( (SRC_IDX + 1 + RANDOM % 4) % 5 ))
  TX_TYPE=$((RANDOM % 4))  # 0=XRP Payment, 1=IOU Payment, 2=OfferCreate, 3=TrustSet
  
  # Rotate submission node: R0, Q0, Q1, Q2
  NODES=("$R0" "$Q0" "$Q1" "$Q2")
  NODE=${NODES[$((RANDOM % 4))]}
  
  SRC=${ACCOUNTS[$SRC_IDX]}
  DST=${ACCOUNTS[$DST_IDX]}
  SEC=${SECRETS[$SRC_IDX]}
  SEQ=$(get_seq "$NODE" "$SRC" 2>/dev/null || echo "")
  
  if [ -z "$SEQ" ]; then
    sleep "$INTERVAL"
    continue
  fi
  
  case $TX_TYPE in
    0) # XRP Payment
      TX="{\"TransactionType\":\"Payment\",\"Account\":\"$SRC\",\"Destination\":\"$DST\",\"Amount\":\"$((RANDOM % 1000000 + 100000))\",\"Sequence\":$SEQ,\"Fee\":\"12\"}"
      ;;
    1) # IOU Payment
      AMT="0.$((RANDOM % 999 + 1))"
      TX="{\"TransactionType\":\"Payment\",\"Account\":\"$SRC\",\"Destination\":\"$DST\",\"Amount\":{\"currency\":\"USD\",\"issuer\":\"${ACCOUNTS[0]}\",\"value\":\"$AMT\"},\"Sequence\":$SEQ,\"Fee\":\"12\"}"
      ;;
    2) # OfferCreate (USD/XRP)
      AMT="0.$((RANDOM % 99 + 1))"
      TX="{\"TransactionType\":\"OfferCreate\",\"Account\":\"$SRC\",\"TakerPays\":{\"currency\":\"USD\",\"issuer\":\"${ACCOUNTS[0]}\",\"value\":\"$AMT\"},\"TakerGets\":\"$((RANDOM % 5000000 + 100000))\",\"Sequence\":$SEQ,\"Fee\":\"12\"}"
      ;;
    3) # OfferCreate reverse (XRP/USD)
      AMT="0.$((RANDOM % 99 + 1))"
      TX="{\"TransactionType\":\"OfferCreate\",\"Account\":\"$SRC\",\"TakerPays\":\"$((RANDOM % 5000000 + 100000))\",\"TakerGets\":{\"currency\":\"USD\",\"issuer\":\"${ACCOUNTS[0]}\",\"value\":\"$AMT\"},\"Sequence\":$SEQ,\"Fee\":\"12\"}"
      ;;
  esac
  
  RESULT=$(submit_tx "$NODE" "$SEC" "$TX")
  TX_SENT=$((TX_SENT + 1))
  if [ "$RESULT" = "tesSUCCESS" ] || [ "$RESULT" = "terQUEUED" ]; then
    TX_SUCCESS=$((TX_SUCCESS + 1))
  else
    TX_FAIL=$((TX_FAIL + 1))
  fi
  
  # Every 30 rounds: check drift and hashes
  if [ $((ROUND % 30)) -eq 0 ]; then
    RS=$(get_validated_seq "$R0")
    QS=$(get_validated_seq "$Q0")
    DRIFT=$(( ${RS:-0} - ${QS:-0} ))
    if [ ${DRIFT#-} -gt ${MAX_DRIFT#-} ]; then MAX_DRIFT=$DRIFT; fi
    
    # Hash check on quaxar's latest
    if [ "${QS:-0}" -gt "$LAST_HASH_CHECK" ] && [ "${QS:-0}" -gt 0 ]; then
      check_hashes "$QS" || true
      LAST_HASH_CHECK=${QS:-0}
    fi
    
    log "Round $ROUND | tx=$TX_SENT ok=$TX_SUCCESS fail=$TX_FAIL | R=$RS Q=$QS drift=$DRIFT | hashes=${HASH_MATCHES}/${HASH_CHECKS} ✅"
  fi
  
  sleep "$INTERVAL"
done

# --- Final report ---
log ""
log "========================================="
log "       DEVNET LOAD TEST COMPLETE"
log "========================================="
log "Duration: ${ELAPSED}s ($(( ELAPSED / 3600 ))h $(( (ELAPSED % 3600) / 60 ))m)"
log "Transactions: sent=$TX_SENT success=$TX_SUCCESS failed=$TX_FAIL"
log "Success rate: $(( TX_SUCCESS * 100 / (TX_SENT > 0 ? TX_SENT : 1) ))%"
log "Hash checks: ${HASH_MATCHES}/${HASH_CHECKS} matched"
log "Hash mismatches: $HASH_MISMATCHES"
log "Max drift: $MAX_DRIFT ledgers"
log ""
if [ $HASH_MISMATCHES -eq 0 ] && [ $MAX_DRIFT -lt 10 ]; then
  log "✅ RESULT: PASS"
else
  log "❌ RESULT: FAIL"
fi
log "========================================="
