#!/usr/bin/env python3
"""
Batch SHAMap parity verification against XRPL mainnet.

Fetches ledgers in batches, writes fixture files, runs quaxar's Rust SHAMap
test against each one, and produces a detailed results log.

Usage:
    python3 scripts/verify_mainnet_batch.py [--count 500] [--batch-size 10]
"""

import json, subprocess, time, sys, os, argparse
from datetime import datetime

RPC = 'https://xrpl.rustychain.eu/rpc'
FIXTURE_PATH = os.path.join(os.path.dirname(__file__), '..', 'xrpl', 'shamap', 'tests', 'fixtures', 'mainnet_ledger.json')
RESULTS_LOG = os.path.join(os.path.dirname(__file__), '..', 'mainnet_parity_results.log')
PROJECT_ROOT = os.path.join(os.path.dirname(__file__), '..')

# ─── RPC Helper ───────────────────────────────────────────────────────────────

def rpc(method, params, retries=4):
    payload = json.dumps({'method': method, 'params': [params]})
    for attempt in range(retries):
        try:
            r = subprocess.run(
                ['curl', '-sk', '--max-time', '25', '-X', 'POST',
                 '-H', 'Content-Type: application/json', '-d', payload, RPC],
                capture_output=True, text=True, timeout=30
            )
            if r.returncode == 0 and r.stdout.strip().startswith('{'):
                result = json.loads(r.stdout).get('result')
                if result and 'error' not in result:
                    return result
        except subprocess.TimeoutExpired:
            pass
        time.sleep(1 + attempt)
    return None

# ─── Fetch a batch of ledgers ─────────────────────────────────────────────────

def fetch_ledger(seq):
    """Fetch binary+expand and JSON versions of a ledger. Returns fixture dict or None."""
    bin_r = rpc('ledger', {'ledger_index': seq, 'transactions': True, 'binary': True, 'expand': True})
    json_r = rpc('ledger', {'ledger_index': seq})

    if not bin_r or not json_r:
        return None

    ld = json_r['ledger']
    txs = bin_r['ledger'].get('transactions', [])

    return {
        'ledger_index': int(ld['ledger_index']),
        'ledger_hash': ld['ledger_hash'],
        'parent_hash': ld['parent_hash'],
        'transaction_hash': ld['transaction_hash'],
        'account_hash': ld['account_hash'],
        'total_coins': ld['total_coins'],
        'close_time': ld['close_time'],
        'parent_close_time': ld['parent_close_time'],
        'close_time_resolution': ld.get('close_time_resolution', 10),
        'close_flags': ld.get('close_flags', 0),
        'transactions': [{'tx_blob': t['tx_blob'], 'meta_blob': t['meta']} for t in txs],
    }

# ─── Run Rust test ────────────────────────────────────────────────────────────

def run_rust_test():
    """Run cargo test for the SHAMap parity test. Returns (passed: bool, output: str)."""
    r = subprocess.run(
        ['cargo', 'test', '-p', 'shamap', '--test', 'mainnet_parity', '--', '--quiet'],
        capture_output=True, text=True,
        cwd=PROJECT_ROOT,
        timeout=60,
    )
    return r.returncode == 0, (r.stdout + r.stderr)[-300:]

# ─── Logging ──────────────────────────────────────────────────────────────────

def log(msg, logfile=None):
    ts = datetime.now().strftime('%H:%M:%S')
    line = f"[{ts}] {msg}"
    print(line, flush=True)
    if logfile:
        logfile.write(line + '\n')
        logfile.flush()

# ─── Main ─────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description='Batch SHAMap mainnet parity verification')
    parser.add_argument('--count', type=int, default=500, help='Number of ledgers to verify')
    parser.add_argument('--batch-size', type=int, default=10, help='Report progress every N ledgers')
    args = parser.parse_args()

    with open(RESULTS_LOG, 'w') as logfile:
        log("╔══════════════════════════════════════════════════════════╗", logfile)
        log("║  BATCH SHAMAP PARITY VERIFICATION (quaxar vs mainnet)   ║", logfile)
        log("╚══════════════════════════════════════════════════════════╝", logfile)
        log("", logfile)

        # Get available range
        info = rpc('server_info', {})
        if not info:
            log("✗ Cannot reach RPC node", logfile)
            sys.exit(1)

        complete = info['info']['complete_ledgers']
        node_start, node_end = int(complete.split('-')[0]), int(complete.split('-')[1])
        available = node_end - node_start + 1
        actual_count = min(args.count, available)
        start_seq = node_end - actual_count + 1

        log(f"RPC node: {RPC}", logfile)
        log(f"Node range: {complete} ({available} ledgers available)", logfile)
        log(f"Verifying: {start_seq} → {node_end} ({actual_count} ledgers)", logfile)
        log(f"Batch report interval: every {args.batch_size} ledgers", logfile)
        log("", logfile)

        # Pre-compile the test binary once
        log("Pre-compiling Rust test binary...", logfile)
        r = subprocess.run(
            ['cargo', 'test', '-p', 'shamap', '--test', 'mainnet_parity', '--no-run'],
            capture_output=True, text=True, cwd=PROJECT_ROOT, timeout=120
        )
        if 'error' in r.stderr and 'warning' not in r.stderr.split('error')[0]:
            log(f"✗ Compilation failed: {r.stderr[-200:]}", logfile)
            sys.exit(1)
        log("✓ Compiled", logfile)
        log("", logfile)

        # Stats
        verified = 0
        failed = 0
        fetch_errors = 0
        total_txs = 0
        start_time = time.time()
        consecutive_failures = 0

        log(f"{'Seq':<12} {'Txs':>5} {'Status':<8} {'Cumulative':>20}", logfile)
        log(f"{'─'*12} {'─'*5} {'─'*8} {'─'*20}", logfile)

        for seq in range(start_seq, node_end + 1):
            # Fetch
            fixture = fetch_ledger(seq)
            if fixture is None:
                fetch_errors += 1
                consecutive_failures += 1
                if consecutive_failures > 10:
                    log(f"✗ 10 consecutive fetch failures at seq={seq}, stopping", logfile)
                    break
                continue
            consecutive_failures = 0

            tx_count = len(fixture['transactions'])
            total_txs += tx_count

            # Write fixture
            os.makedirs(os.path.dirname(FIXTURE_PATH), exist_ok=True)
            with open(FIXTURE_PATH, 'w') as f:
                json.dump(fixture, f)

            # Run Rust test
            passed, output = run_rust_test()

            if passed:
                verified += 1
            else:
                failed += 1
                log(f"  {seq:<12} {tx_count:>5} ✗ FAIL   {output[-100:]}", logfile)

            # Progress report
            done = verified + failed
            if done % args.batch_size == 0 or seq == node_end:
                elapsed = time.time() - start_time
                rate = done / elapsed if elapsed > 0 else 0
                remaining = actual_count - done - fetch_errors
                eta = remaining / rate if rate > 0 else 0
                log(f"  {seq:<12} {tx_count:>5} {'✓' if passed else '✗':>8} "
                    f"v={verified} f={failed} txs={total_txs} "
                    f"{rate:.1f}/s ETA={eta/60:.1f}min", logfile)

            time.sleep(0.15)  # gentle rate limit

        # Final report
        elapsed = time.time() - start_time
        log("", logfile)
        log("═" * 60, logfile)
        log("  FINAL RESULTS", logfile)
        log("═" * 60, logfile)
        log(f"  Ledgers verified (SHAMap root match): {verified}", logfile)
        log(f"  Ledgers failed:                       {failed}", logfile)
        log(f"  Fetch errors (skipped):               {fetch_errors}", logfile)
        log(f"  Total transactions through SHAMap:    {total_txs}", logfile)
        log(f"  Duration:                             {elapsed:.0f}s ({elapsed/60:.1f}min)", logfile)
        log(f"  Rate:                                 {verified/(elapsed or 1):.2f} ledgers/s", logfile)
        log("", logfile)

        if failed == 0 and verified > 0:
            log(f"  ✓ ALL {verified} LEDGERS PASS", logfile)
            log(f"    quaxar's SHAMap code produces bit-identical transaction tree", logfile)
            log(f"    hashes to XRPL mainnet ({total_txs} transactions verified)", logfile)
        elif failed > 0:
            log(f"  ✗ {failed} FAILURES DETECTED", logfile)
        else:
            log(f"  ⚠ No ledgers could be verified (all fetch failures)", logfile)

        log("", logfile)
        log(f"  Results saved to: {RESULTS_LOG}", logfile)

if __name__ == '__main__':
    main()
