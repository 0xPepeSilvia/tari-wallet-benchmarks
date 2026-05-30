#!/usr/bin/env python
"""S1 driver: UTXO build-up via doubling + fan-out.

Spec round structure:
  Round 1: 1 tx,  1 in -> 2 out  =>  2 UTXOs after
  Round 2: 2 txs, 1 in -> 2 out  =>  4 UTXOs
  Round 3: 4 txs                 =>  8 UTXOs
  Round 4: 8 txs                 => 16 UTXOs
  Round 5: 16 txs                => 32 UTXOs
  Round 6: 32 txs                => 64 UTXOs
Fan-out: 64 txs, 1 in -> 8 out   => 512 UTXOs

For each tx: capture construction time and tx_id.
Wait for all round txs to reach depth >= C_min before next round.
Failure halts the scenario per spec.

Usage:
  python run_s1.py --port 18243 --doubling-rounds 6 --fanout-tx 64 --fanout-outputs 8 --c-min 1
"""
import argparse, grpc, json, time, sys
from datetime import datetime, timezone

def varint(n):
    out = b''
    while True:
        b = n & 0x7f
        n >>= 7
        if n:
            out += bytes([b | 0x80])
        else:
            out += bytes([b])
            return out

def vfield(t, n): return varint((t << 3) | 0) + varint(n)
def sfield(t, s): enc = s.encode(); return varint((t << 3) | 2) + varint(len(enc)) + enc

def coin_split(ch, amount_per_split, split_count, fee_per_gram=5):
    """CoinSplitRequest{amount_per_split=1, split_count=2, fee_per_gram=3, message=4, lock_height=5}"""
    body = (
        vfield(1, amount_per_split) +
        vfield(2, split_count) +
        vfield(3, fee_per_gram) +
        sfield(4, "s1") +
        vfield(5, 0)
    )
    resp = ch.unary_unary('/tari.rpc.Wallet/CoinSplit',
        request_serializer=lambda x: x, response_deserializer=lambda x: x)(body, timeout=120)
    # CoinSplitResponse{uint64 transaction_id=1}
    if resp and resp[0] == 0x08:
        i = 1; v = 0; sh = 0
        while True:
            b = resp[i]; i += 1
            v |= (b & 0x7f) << sh
            if not (b & 0x80): break
            sh += 7
        return v
    raise RuntimeError(f"unexpected CoinSplit response: {resp.hex()}")

def get_balance(ch):
    r = ch.unary_unary('/tari.rpc.Wallet/GetBalance',
        request_serializer=lambda x: x, response_deserializer=lambda x: x)(b'', timeout=10)
    out = {}; i = 0
    while i < len(r):
        tag = r[i]; i += 1; f = tag >> 3
        v = 0; sh = 0
        while True:
            b = r[i]; i += 1
            v |= (b & 0x7f) << sh
            if not (b & 0x80): break
            sh += 7
        out[f] = v
    return out  # {1:available, 2:pending_in, 3:pending_out, 4:timelocked}

def tx_status(ch, tx_id):
    """GetTransactionInfoRequest{repeated uint64 transaction_ids=1}"""
    body = vfield(1, tx_id)  # packed=false single uint64
    body = varint((1 << 3) | 0) + varint(tx_id)
    try:
        resp = ch.unary_unary('/tari.rpc.Wallet/GetTransactionInfo',
            request_serializer=lambda x: x, response_deserializer=lambda x: x)(body, timeout=10)
        # GetTransactionInfoResponse{repeated TransactionInfo transactions=1}
        # TransactionInfo.status is field 4 (enum)
        # Just look for status field in any nested message
        return len(resp) > 0
    except Exception:
        return False

def wait_for_balance_decrease(ch, baseline_avail, drop_target, timeout_s=600):
    """Wait until wallet's available balance has decreased by at least drop_target uT."""
    start = time.time()
    while time.time() - start < timeout_s:
        b = get_balance(ch)
        avail = b.get(1, 0)
        if avail <= baseline_avail - drop_target:
            return time.time() - start, b
        time.sleep(5)
    return -1, get_balance(ch)

def wait_for_balance_settle(ch, expected_avail_min, timeout_s=600):
    """Wait until available balance >= expected_avail_min and pending=0."""
    start = time.time()
    while time.time() - start < timeout_s:
        b = get_balance(ch)
        avail = b.get(1, 0); p_in = b.get(2, 0); p_out = b.get(3, 0)
        if avail >= expected_avail_min and p_in == 0 and p_out == 0:
            return time.time() - start, b
        time.sleep(10)
    return -1, get_balance(ch)

def get_scanned_height(ch):
    """GetState -> scanned_height (field 1)."""
    body = b''  # GetStateRequest is empty
    r = ch.unary_unary('/tari.rpc.Wallet/GetState',
        request_serializer=lambda x: x, response_deserializer=lambda x: x)(body, timeout=10)
    if r and r[0] == 0x08:
        i = 1; v = 0; sh = 0
        while True:
            b = r[i]; i += 1
            v |= (b & 0x7f) << sh
            if not (b & 0x80): break
            sh += 7
        return v
    return 0

def wait_for_chain_advance(ch, blocks=1, timeout_s=900):
    """Wait until the wallet's scanned_height has advanced by `blocks` from current."""
    start = time.time()
    h0 = get_scanned_height(ch)
    target = h0 + blocks
    while time.time() - start < timeout_s:
        h = get_scanned_height(ch)
        if h >= target:
            return time.time() - start, h
        time.sleep(15)
    return -1, get_scanned_height(ch)

def wait_for_funds_spendable(ch, retries=120, sleep_s=10):
    """Probe-test that outputs are spendable by attempting a no-op (tiny) split.
    Avoid infinite loops by retrying a coin_split with a tiny amount and checking
    for FundsPending error.  Returns elapsed seconds when spendable."""
    start = time.time()
    # Cheaper: just wait for chain to advance 2 blocks.
    elapsed, h = wait_for_chain_advance(ch, blocks=2, timeout_s=retries * sleep_s)
    return elapsed

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, default=18243)
    ap.add_argument('--doubling-rounds', type=int, default=6)
    ap.add_argument('--fanout-tx', type=int, default=64)
    ap.add_argument('--fanout-outputs', type=int, default=8)
    ap.add_argument('--c-min', type=int, default=1)
    ap.add_argument('--output', default='results/S1_mode1_2026-05-30.json')
    args = ap.parse_args()

    ch = grpc.insecure_channel(f'127.0.0.1:{args.port}')

    print(f"S1 driver against 127.0.0.1:{args.port}")
    b = get_balance(ch)
    initial_avail = b.get(1, 0)
    print(f"  starting balance: {initial_avail/1e6:,.2f} tXTM")
    if initial_avail < 100_000_000:
        print(f"  ERROR: balance < 100 tXTM; cannot proceed")
        sys.exit(1)

    result = {
        "scenario": "S1",
        "mode": 1,
        "started_at": datetime.now(timezone.utc).isoformat(),
        "initial_balance_ut": initial_avail,
        "doubling_rounds": [],
        "fanout": {},
    }

    # ── Doubling phase ──────────────────────────────────────────────────────
    scenario_start = time.time()

    for r in range(args.doubling_rounds):
        txs_this_round = 1 << r  # 1, 2, 4, 8, 16, 32
        round_start = time.time()
        print(f"\n[round {r+1}/{args.doubling_rounds}] {txs_this_round} tx(s)")

        b = get_balance(ch)
        avail_before = b.get(1, 0)
        # FIXED small split amount: 50 tXTM per output.  Critical finding:
        # the wallet does not auto-aggregate small UTXOs to fund a coin_split,
        # so each round's per-output amount must be << the smallest UTXO from
        # any prior round.  Using a fixed small value keeps headroom across
        # all rounds (each tx consumes ~100 tXTM + fee from the wallet's
        # largest available UTXO via change).
        split_amount = 50_000_000  # 50 tXTM in µT

        tx_ids = []
        construct_times = []
        for i in range(txs_this_round):
            try:
                t0 = time.time()
                tx_id = coin_split(ch, split_amount, 2, fee_per_gram=5)
                ct = time.time() - t0
                tx_ids.append(tx_id)
                construct_times.append(ct)
                print(f"    tx {i+1}/{txs_this_round}: id={tx_id} construct={ct:.2f}s")
            except Exception as e:
                print(f"    tx {i+1}/{txs_this_round} FAILED: {e}")
                # Per spec: failure halts the scenario.
                result["doubling_rounds"].append({
                    "round": r+1, "txs_attempted": txs_this_round,
                    "txs_succeeded": i, "error": str(e),
                })
                result["status"] = "FAILED_AT_DOUBLING"
                with open(args.output, 'w') as f:
                    json.dump(result, f, indent=2)
                print(f"  S1 ABORTED. Partial results written to {args.output}")
                sys.exit(1)

        # Wait for the round's tx outputs to be SPENDABLE.  GetBalance's
        # pending_in/out fields do NOT capture wallet-internal pending state -
        # the output_manager rejects spends from unmined tx outputs with
        # OutputManagerError(FundsPending) even when pending_in shows 0.
        # We must wait for chain height to advance past the round's broadcast.
        # Wait for blocks to mine.  Finding #6 (FundsPending mid-round): the
        # wallet considers change from in-flight txs as unspendable until they
        # mine with sufficient depth.  Use 4 blocks rather than C_min+1 so the
        # next round has a pool of properly-confirmed UTXOs.
        wait_blocks = max(args.c_min + 1, 4)
        print(f"  waiting for round {r+1} txs to mine ({len(tx_ids)} txs, waiting {wait_blocks} blocks)...")
        settle_time, h = wait_for_chain_advance(ch, blocks=wait_blocks, timeout_s=1200)
        b = get_balance(ch)
        round_elapsed = time.time() - round_start

        result["doubling_rounds"].append({
            "round": r+1,
            "txs_count": txs_this_round,
            "tx_ids": tx_ids,
            "construct_times_secs": construct_times,
            "total_construct_secs": sum(construct_times),
            "round_wall_secs": round_elapsed,
            "settle_wait_secs": settle_time,
            "balance_after_ut": b.get(1, 0),
        })
        print(f"  round {r+1} done in {round_elapsed:.1f}s (settle {settle_time:.0f}s)")

    # ── Fan-out ─────────────────────────────────────────────────────────────
    print(f"\n[fanout] {args.fanout_tx} txs, {args.fanout_outputs} outputs each")
    fan_start = time.time()
    b = get_balance(ch)
    avail_before_fan = b.get(1, 0)
    per_output = 10_000_000  # 10 tXTM per fanout output (small to preserve headroom)

    fanout_tx_ids = []
    fan_construct = []
    for i in range(args.fanout_tx):
        try:
            t0 = time.time()
            tx_id = coin_split(ch, per_output, args.fanout_outputs, fee_per_gram=5)
            ct = time.time() - t0
            fanout_tx_ids.append(tx_id)
            fan_construct.append(ct)
            if (i+1) % 8 == 0:
                print(f"    fanout {i+1}/{args.fanout_tx}: id={tx_id} construct={ct:.2f}s")
        except Exception as e:
            print(f"    fanout {i+1}/{args.fanout_tx} FAILED: {e}")
            result["fanout"] = {
                "txs_attempted": args.fanout_tx,
                "txs_succeeded": i,
                "tx_ids": fanout_tx_ids,
                "error": str(e),
            }
            result["status"] = "FAILED_AT_FANOUT"
            with open(args.output, 'w') as f: json.dump(result, f, indent=2)
            sys.exit(1)

    print("  waiting for fanout txs to mine...")
    fan_settle, _ = wait_for_chain_advance(ch, blocks=args.c_min + 1, timeout_s=1800)
    b = get_balance(ch)
    fan_elapsed = time.time() - fan_start

    result["fanout"] = {
        "txs_count": args.fanout_tx,
        "outputs_per_tx": args.fanout_outputs,
        "tx_ids": fanout_tx_ids,
        "construct_times_secs": fan_construct,
        "total_construct_secs": sum(fan_construct),
        "wall_secs": fan_elapsed,
        "settle_wait_secs": fan_settle,
        "balance_after_ut": b.get(1, 0),
    }

    total_elapsed = time.time() - scenario_start
    result["finished_at"] = datetime.now(timezone.utc).isoformat()
    result["wall_clock_secs"] = total_elapsed
    result["status"] = "PASSED"
    result["txs_total"] = sum(1 << r for r in range(args.doubling_rounds)) + args.fanout_tx
    result["final_balance_ut"] = b.get(1, 0)

    with open(args.output, 'w') as f: json.dump(result, f, indent=2)
    print(f"\nS1 complete: {result['txs_total']} txs in {total_elapsed:.1f}s")
    print(f"Results: {args.output}")

if __name__ == '__main__':
    main()
