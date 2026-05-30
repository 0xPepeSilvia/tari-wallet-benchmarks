#!/usr/bin/env python
"""S4 driver: Concurrent transaction construction.

Spec: for each N in {8, 16, 32, 64, 128}:
  Fire N concurrent CoinSplit (or Transfer) gRPC calls in parallel.
  Record per-tx: submit time, construction-complete time, outcome, error.
  Wait for all terminal states OR T_budget elapsed.
  NO retry, NO backoff, NO UTXO pre-partitioning.

The signal we want is: locking gaps, serialisation gaps, double-selection
rejections, FundsPending bursts.
"""
import argparse, grpc, json, time, threading, sys
from datetime import datetime, timezone
from concurrent.futures import ThreadPoolExecutor, as_completed

def varint(n):
    out = b''
    while True:
        b = n & 0x7f; n >>= 7
        if n: out += bytes([b | 0x80])
        else: out += bytes([b]); return out

def vf(t, n): return varint((t<<3)|0) + varint(n)
def sf(t, s): e = s.encode(); return varint((t<<3)|2) + varint(len(e)) + e

def coin_split(channel, amount_per_split, count, fee=5):
    """Build CoinSplitRequest, send via raw gRPC."""
    body = vf(1, amount_per_split) + vf(2, count) + vf(3, fee) + sf(4, "s4") + vf(5, 0)
    return channel.unary_unary(
        '/tari.rpc.Wallet/CoinSplit',
        request_serializer=lambda x: x,
        response_deserializer=lambda x: x,
    )(body, timeout=120)

def worker(idx, channel, amount):
    """Run one CoinSplit call, return timing + outcome."""
    t_submit = time.perf_counter()
    try:
        resp = coin_split(channel, amount, 2, fee=5)
        t_complete = time.perf_counter()
        # parse tx_id
        tx_id = None
        if resp and resp[0] == 0x08:
            i = 1; v = 0; sh = 0
            while True:
                b = resp[i]; i += 1
                v |= (b & 0x7f) << sh
                if not (b & 0x80): break
                sh += 7
            tx_id = v
        return {
            "worker_idx": idx,
            "t_submit": t_submit,
            "t_complete": t_complete,
            "construct_secs": t_complete - t_submit,
            "tx_id": tx_id,
            "outcome": "ok",
            "error": None,
        }
    except grpc.RpcError as e:
        t_complete = time.perf_counter()
        details = e.details() if hasattr(e, 'details') else str(e)
        outcome = "rejected"
        if "FundsPending" in details: outcome = "funds_pending"
        elif "NotEnoughFunds" in details: outcome = "not_enough_funds"
        elif "AlreadySelected" in details or "double" in details.lower(): outcome = "double_selection"
        return {
            "worker_idx": idx,
            "t_submit": t_submit,
            "t_complete": t_complete,
            "construct_secs": t_complete - t_submit,
            "tx_id": None,
            "outcome": outcome,
            "error": details,
        }
    except Exception as e:
        return {
            "worker_idx": idx,
            "t_submit": t_submit,
            "t_complete": time.perf_counter(),
            "construct_secs": 0,
            "tx_id": None,
            "outcome": "exception",
            "error": str(e),
        }

def run_level(port, N, amount_ut, budget_s):
    """Run one N-concurrency level."""
    print(f"\n--- N={N} ---", flush=True)
    # Each worker gets its own channel for true concurrent gRPC.
    channels = [grpc.insecure_channel(f'127.0.0.1:{port}') for _ in range(N)]

    t_start = time.perf_counter()
    results = []
    with ThreadPoolExecutor(max_workers=N) as ex:
        futures = {ex.submit(worker, i, channels[i], amount_ut): i for i in range(N)}
        try:
            for f in as_completed(futures, timeout=budget_s):
                results.append(f.result())
        except TimeoutError:
            print(f"  budget {budget_s}s exhausted", flush=True)

    t_end = time.perf_counter()
    wall = t_end - t_start

    # Compute serialisation gap: max gap between consecutive t_complete values
    # sorted by t_complete.  Big gaps = something held the wallet up.
    completed = sorted([r["t_complete"] for r in results])
    max_gap = 0.0
    for i in range(1, len(completed)):
        max_gap = max(max_gap, completed[i] - completed[i-1])

    successes = [r for r in results if r["outcome"] == "ok"]
    fail_kinds = {}
    for r in results:
        if r["outcome"] != "ok":
            fail_kinds[r["outcome"]] = fail_kinds.get(r["outcome"], 0) + 1

    txs_per_sec = len(successes) / wall if wall > 0 else 0
    print(f"  ok={len(successes)}/{N}, wall={wall:.2f}s, tx/s={txs_per_sec:.1f}, max_gap={max_gap*1000:.0f}ms", flush=True)
    if fail_kinds:
        print(f"  failures: {fail_kinds}", flush=True)

    return {
        "N": N,
        "amount_per_output_ut": amount_ut,
        "wall_secs": wall,
        "txs_constructed_ok": len(successes),
        "txs_per_sec_observed": txs_per_sec,
        "max_serialisation_gap_secs": max_gap,
        "failure_breakdown": fail_kinds,
        "per_worker_results": results,
    }

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, default=18143)
    ap.add_argument('--levels', type=str, default='8,16,32,64,128')
    ap.add_argument('--budget-secs', type=int, default=900)
    ap.add_argument('--amount-ut', type=int, default=1_000_000)  # 1 tXTM per output
    ap.add_argument('--output', default='results/S4_mode1_2026-05-30.json')
    args = ap.parse_args()

    levels = [int(x) for x in args.levels.split(',')]
    print(f"S4 against 127.0.0.1:{args.port}; levels={levels}; budget={args.budget_secs}s", flush=True)

    out = {
        "scenario": "S4",
        "mode": 1,
        "endpoint": f"http://127.0.0.1:{args.port}",
        "started_at": datetime.now(timezone.utc).isoformat(),
        "config": {
            "concurrency_levels": levels,
            "budget_secs": args.budget_secs,
            "amount_per_output_ut": args.amount_ut,
        },
        "levels": [],
    }

    for N in levels:
        result = run_level(args.port, N, args.amount_ut, args.budget_secs)
        out["levels"].append(result)
        # Brief pause between levels to let wallet recover.
        time.sleep(10)

    out["finished_at"] = datetime.now(timezone.utc).isoformat()
    with open(args.output, 'w') as f:
        json.dump(out, f, indent=2, default=str)
    print(f"\nresults written to {args.output}", flush=True)

if __name__ == '__main__':
    main()
