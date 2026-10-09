#!/usr/bin/env bash
# Matched RPC/WS benchmark: rippled vs quaxar baseline vs quaxar (this
# branch), all in containers (infra/bench/docker-compose.yml).
#
# Each server runs standalone, alone, with 4 CPUs / 4 GiB; it is populated
# with the same ledger (scripts/bench_populate.py, signed offline by
# rippled) and driven by the same closed-loop client
# (xrpld/server/examples/rpc_load.rs). Results: infra/bench/results.jsonl.
#
# Usage: scripts/bench_matrix.sh [servers...]   (default: all three)
set -euo pipefail
cd "$(dirname "$0")/../infra/bench"
C="docker compose -f docker-compose.yml -p quaxar-bench"
OUT=results.jsonl
SERVERS=("${@:-rippled quaxar-base quaxar-new}")
read -r -a SERVERS <<<"${SERVERS[*]}"

lg() { $C exec -T loadgen "$@"; }

wait_ready() {
  for _ in $(seq 1 90); do
    if lg curl -sf -m 3 "http://$1:5005/" -d '{"method":"server_info","params":[{}]}' >/dev/null; then
      return 0
    fi
    sleep 2
  done
  echo "$1 did not become ready" >&2
  exit 1
}

$C up -d rippled loadgen
wait_ready rippled
lg cargo build --release -q -p server --example rpc_load
LOAD=/target/release/examples/rpc_load
: >"$OUT"

for server in "${SERVERS[@]}"; do
  if [ "$server" != rippled ]; then
    $C up -d "$server"
  fi
  wait_ready "$server"
  state=$(lg python3 /src/scripts/bench_populate.py "http://$server:5005/" "http://rippled:5005/")
  gateway=$(jq -r .gateway <<<"$state")
  holder=$(jq -r .holder <<<"$state")
  sample=$(jq -r .sample_tx <<<"$state")
  echo "$server populated: $state" >&2

  # name | request (command form) | requests per connection
  requests=(
    "ping|{\"command\":\"ping\"}|3000"
    "fee|{\"command\":\"fee\"}|2000"
    "account_info|{\"command\":\"account_info\",\"account\":\"$gateway\",\"ledger_index\":\"validated\"}|2000"
    "tx|{\"command\":\"tx\",\"transaction\":\"$sample\"}|1000"
    "account_objects_200|{\"command\":\"account_objects\",\"account\":\"$holder\",\"limit\":400,\"ledger_index\":\"validated\"}|150"
    "book_offers_200|{\"command\":\"book_offers\",\"taker_pays\":{\"currency\":\"XRP\"},\"taker_gets\":{\"currency\":\"USD\",\"issuer\":\"$gateway\"},\"limit\":200,\"ledger_index\":\"validated\"}|100"
    "ledger_data_256|{\"command\":\"ledger_data\",\"limit\":256,\"ledger_index\":\"validated\"}|100"
    "account_tx_200|{\"command\":\"account_tx\",\"account\":\"$gateway\",\"limit\":200}|100"
  )
  for entry in "${requests[@]}"; do
    IFS='|' read -r name request reqs <<<"$entry"
    for transport in http ws; do
      port=5005
      [ "$transport" = ws ] && port=6006
      for conns in 1 16; do
        per=$reqs
        [ "$conns" = 16 ] && per=$(( reqs / 4 > 20 ? reqs / 4 : 20 ))
        result=$(lg "$LOAD" "$transport://$server:$port/" "$request" "$conns" "$per")
        line=$(jq -c --arg s "$server" --arg m "$name" --arg t "$transport" --argjson c "$conns" \
          '. + {server:$s, method:$m, transport:$t, conns:$c}' <<<"$result")
        echo "$line" | tee -a "$OUT"
      done
    done
  done

  if [ "$server" != rippled ]; then
    $C stop "$server"
  fi
done
$C down
