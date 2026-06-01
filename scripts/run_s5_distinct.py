#!/usr/bin/env python
"""S5 batch vs individual with M distinct recipient addresses.

Same shape as run_s5.py but reads recipient addresses from a JSON
file produced by gen_distinct_addresses.py instead of self-sending.

This is the rigorous version of S5 that doesn't let change-output reuse
hide the wallet's behavior on distinct-address sends.
"""
import argparse, grpc, json, time
from datetime import datetime, timezone

def varint(n):
    out = b''
    while True:
        b = n & 0x7f; n >>= 7
        if n: out += bytes([b | 0x80])
        else: out += bytes([b]); return out
def vf(t, n): return varint((t<<3)|0) + varint(n)
def sf(t, s): e = s.encode(); return varint((t<<3)|2) + varint(len(e)) + e
def mf(t, body): return varint((t<<3)|2) + varint(len(body)) + body

def make_recipient(address, amount, fee=5, payment_type=2):
    return sf(1, address) + vf(2, amount) + vf(3, fee) + sf(4, "s5_distinct") + vf(5, payment_type)

def transfer(ch, recipients_payload):
    body = b''
    for rp in recipients_payload:
        body += mf(1, rp)
    return ch.unary_unary('/tari.rpc.Wallet/Transfer',
        request_serializer=lambda x: x, response_deserializer=lambda x: x)(body, timeout=180)

def parse_transfer_results(resp):
    results = []
    i = 0
    while i < len(resp):
        tag = resp[i]; i += 1; w = tag & 7
        if w == 2:
            length = 0; sh = 0
            while True:
                b = resp[i]; i += 1
                length |= (b & 0x7f) << sh
                if not (b & 0x80): break
                sh += 7
            payload = resp[i:i+length]; i += length
            j = 0; tx_id = None; ok = None; msg = ""
            while j < len(payload):
                tb = payload[j]; j += 1
                fn = tb >> 3; ww = tb & 7
                if ww == 0:
                    v = 0; sh = 0
                    while True:
                        b = payload[j]; j += 1
                        v |= (b & 0x7f) << sh
                        if not (b & 0x80): break
                        sh += 7
                    if fn == 2: tx_id = v
                    elif fn == 3: ok = bool(v)
                elif ww == 2:
                    sl = 0; sh = 0
                    while True:
                        b = payload[j]; j += 1
                        sl |= (b & 0x7f) << sh
                        if not (b & 0x80): break
                        sh += 7
                    pv = payload[j:j+sl]; j += sl
                    if fn == 4: msg = pv.decode('utf-8', errors='replace')
            results.append((tx_id, ok, msg))
    return results

def get_balance(ch):
    r = ch.unary_unary('/tari.rpc.Wallet/GetBalance', request_serializer=lambda x:x, response_deserializer=lambda x:x)(b'', timeout=10)
    f = {}; i = 0
    while i < len(r):
        tag = r[i]; i += 1; fn = tag >> 3
        v = 0; sh = 0
        while True:
            b = r[i]; i += 1
            v |= (b & 0x7f) << sh
            if not (b & 0x80): break
            sh += 7
        f[fn] = v
    return f

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, default=18243)
    ap.add_argument('--M', type=int, default=100)
    ap.add_argument('--K', type=int, default=10)
    ap.add_argument('--amount-ut', type=int, default=1_000_000)
    ap.add_argument('--addresses', default='results/distinct_addresses_100.json')
    ap.add_argument('--output', default='results/S5_distinct_2026-05-31.json')
    args = ap.parse_args()

    with open(args.addresses) as f:
        addr_data = json.load(f)
    addresses = addr_data['addresses']
    assert len(addresses) >= args.M, f"need at least {args.M} addresses, have {len(addresses)}"
    print(f"using {args.M} distinct addresses (out of {len(addresses)} available)", flush=True)

    ch = grpc.insecure_channel(f'127.0.0.1:{args.port}')
    pre = get_balance(ch)
    print(f"pre-test balance: {pre.get(1,0)/1e6:.2f} tXTM available", flush=True)

    out = {
        "scenario": "S5_distinct_recipients",
        "mode": 1,
        "endpoint": f"http://127.0.0.1:{args.port}",
        "started_at": datetime.now(timezone.utc).isoformat(),
        "config": {"M": args.M, "K": args.K, "amount_per_recipient_ut": args.amount_ut,
                    "recipients_distinct": True, "addresses_source": args.addresses},
        "balance_before_ut": pre.get(1, 0),
    }

    # Arm A: batch (K recipients per tx) — sliced from the distinct list
    batch_count = args.M // args.K
    print(f"\n=== Arm A (batch): {batch_count} txs of {args.K} distinct recipients each ===", flush=True)
    a_start = time.perf_counter()
    a_per_call = []
    a_results = []
    for i in range(batch_count):
        start_idx = i * args.K
        slice_addrs = addresses[start_idx:start_idx + args.K]
        recipients = [make_recipient(a, args.amount_ut) for a in slice_addrs]
        t0 = time.perf_counter()
        try:
            resp = transfer(ch, recipients)
            t1 = time.perf_counter()
            parsed = parse_transfer_results(resp)
            a_per_call.append(t1 - t0)
            ok_count = sum(1 for _, ok, _ in parsed if ok)
            print(f"  batch tx {i+1}/{batch_count}: {ok_count}/{len(parsed)} ok, {(t1-t0)*1000:.0f}ms", flush=True)
            a_results.append({"call_idx": i, "elapsed_secs": t1-t0, "results": parsed,
                              "recipients_used": slice_addrs[:3] + ['...']})  # log first 3
        except Exception as e:
            t1 = time.perf_counter()
            print(f"  batch tx {i+1}/{batch_count} FAILED: {e}", flush=True)
            a_results.append({"call_idx": i, "elapsed_secs": t1-t0, "error": str(e)})
            a_per_call.append(t1 - t0)
    a_wall = time.perf_counter() - a_start
    print(f"Arm A total: {a_wall:.2f}s", flush=True)
    out["arm_a_batch"] = {
        "batch_count": batch_count, "outputs_per_tx": args.K,
        "wall_secs": a_wall, "per_call_secs": a_per_call, "results": a_results,
    }

    print("\nwaiting 60s between arms for pending state to clear...", flush=True)
    time.sleep(60)

    # Arm B: individual — one distinct address per tx
    print(f"\n=== Arm B (individual): {args.M} txs to distinct recipients ===", flush=True)
    b_start = time.perf_counter()
    b_per_call = []
    b_results = []
    for i in range(args.M):
        recipient = [make_recipient(addresses[i], args.amount_ut)]
        t0 = time.perf_counter()
        try:
            resp = transfer(ch, recipient)
            t1 = time.perf_counter()
            parsed = parse_transfer_results(resp)
            b_per_call.append(t1 - t0)
            ok = parsed[0][1] if parsed else None
            b_results.append({"call_idx": i, "elapsed_secs": t1-t0,
                              "tx_id": parsed[0][0] if parsed else None, "ok": ok,
                              "msg": parsed[0][2] if parsed else None})
            if (i+1) % 10 == 0 or not ok:
                tag = "ok" if ok else f"FAIL: {parsed[0][2] if parsed else 'none'}"
                print(f"  indiv {i+1}/{args.M}: {(t1-t0)*1000:.0f}ms {tag}", flush=True)
        except Exception as e:
            t1 = time.perf_counter()
            print(f"  indiv {i+1}/{args.M} FAILED: {e}", flush=True)
            b_results.append({"call_idx": i, "elapsed_secs": t1-t0, "error": str(e)})
            b_per_call.append(t1 - t0)
    b_wall = time.perf_counter() - b_start
    print(f"Arm B total: {b_wall:.2f}s", flush=True)
    out["arm_b_individual"] = {
        "tx_count": args.M, "wall_secs": b_wall, "per_call_secs": b_per_call, "results": b_results,
    }

    speedup = b_wall / a_wall if a_wall > 0 else 0
    out["headline"] = {
        "throughput_multiplier_individual_over_batch": speedup,
        "batch_avg_call_secs": sum(a_per_call) / len(a_per_call) if a_per_call else 0,
        "individual_avg_call_secs": sum(b_per_call) / len(b_per_call) if b_per_call else 0,
        "recipients_distinct": True,
    }
    out["finished_at"] = datetime.now(timezone.utc).isoformat()
    out["balance_after_ut"] = get_balance(ch).get(1, 0)

    with open(args.output, 'w') as f:
        json.dump(out, f, indent=2, default=str)

    print(f"\n=== S5 distinct-recipients headline ===", flush=True)
    print(f"Arm A (batch):      {a_wall:.2f}s for {batch_count} txs × {args.K} outputs", flush=True)
    print(f"Arm B (individual): {b_wall:.2f}s for {args.M} txs × 1 output", flush=True)
    print(f"Speedup (B/A):      {speedup:.2f}×", flush=True)
    print(f"\nresults: {args.output}", flush=True)

if __name__ == '__main__':
    main()
