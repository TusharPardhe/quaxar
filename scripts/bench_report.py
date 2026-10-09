#!/usr/bin/env python3
"""Render infra/bench/results.jsonl (scripts/bench_matrix.sh) as Markdown.

Compares quaxar (this branch) with the best xrpld configuration measured for
each cell: `rippled` (standalone) and `rippled-net` (one-validator network,
multi-threaded JobQueue).
"""
import json
import sys

path = sys.argv[1] if len(sys.argv) > 1 else "infra/bench/results.jsonl"
rows = [json.loads(line) for line in open(path) if line.strip()]
labels = {"rippled": "xrpld standalone", "rippled-net": "xrpld network",
          "quaxar-new": "quaxar"}
servers = [s for s in labels if any(r["server"] == s for r in rows)]
cells = {(r["method"], r["transport"], r["conns"], r["server"]): r for r in rows}
keys = sorted({(r["method"], r["transport"], r["conns"]) for r in rows})


def fmt(r):
    return f'{r["p50_us"]/1000:.2f} / {r["p99_us"]/1000:.2f} / {r["rps"]:,.0f}' if r else "-"


print("| method | transport | conns | " + " | ".join(labels[s] for s in servers)
      + " | quaxar vs best xrpld |")
print("|---|---|---|" + "---|" * len(servers) + "---|")
ratios = []
for k in keys:
    got = {s: cells.get(k + (s,)) for s in servers}
    xrpld = [got[s] for s in ("rippled", "rippled-net") if got.get(s)]
    ratio = "-"
    if xrpld and got.get("quaxar-new"):
        best = max(r["rps"] for r in xrpld)
        value = got["quaxar-new"]["rps"] / best
        ratios.append(value)
        ratio = f"{value:.1f}x"
    print(f"| {k[0]} | {k[1]} | {k[2]} | " + " | ".join(fmt(got[s]) for s in servers)
          + f" | {ratio} |")
bad = [r for r in rows if not r["ok"]]
print(f"\nCells: p50 ms / p99 ms / requests per second. The ratio divides quaxar's "
      f"throughput by the faster xrpld configuration for that cell"
      + (f" (range {min(ratios):.2f}x to {max(ratios):.1f}x)." if ratios else ".")
      + f" Responses with an error member: {len(bad)}.")
