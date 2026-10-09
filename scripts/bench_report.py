#!/usr/bin/env python3
"""Render infra/bench/results.jsonl (scripts/bench_matrix.sh) as Markdown."""
import json
import sys

path = sys.argv[1] if len(sys.argv) > 1 else "infra/bench/results.jsonl"
rows = [json.loads(line) for line in open(path) if line.strip()]
servers = [s for s in ["rippled", "quaxar-base", "quaxar-new", "quaxar-native"]
           if any(r["server"] == s for r in rows)]
cells = {}
for r in rows:
    cells[(r["method"], r["transport"], r["conns"], r["server"])] = r
keys = sorted({(r["method"], r["transport"], r["conns"]) for r in rows})

def fmt(r):
    return f'{r["p50_us"]/1000:.2f} / {r["p99_us"]/1000:.2f} / {r["rps"]:,.0f}' if r else "-"

print("| method | transport | conns | " + " | ".join(servers)
      + " | new vs rippled | new vs base |")
print("|---|---|---|" + "---|" * len(servers) + "---|---|")
for k in keys:
    got = {s: cells.get(k + (s,)) for s in servers}
    def ratio(a, b):
        if got.get(a) and got.get(b):
            return f'{got[a]["rps"] / got[b]["rps"]:.1f}x'
        return "-"
    print(f"| {k[0]} | {k[1]} | {k[2]} | " + " | ".join(fmt(got[s]) for s in servers)
          + f" | {ratio('quaxar-new', 'rippled')} | {ratio('quaxar-new', 'quaxar-base')} |")
bad = [r for r in rows if not r["ok"]]
print(f"\nCells: p50 ms / p99 ms / requests per second. Ratios compare throughput."
      f" Responses with an error member: {len(bad)}.")
