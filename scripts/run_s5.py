#!/usr/bin/env python
"""S5 driver: Payment processor throughput - batch vs individual.

Spec:
  Arm A (batch):      Submit M/K = 10 batch txs (1 input -> K = 10 outputs), back-to-back
  Arm B (individual): Submit M = 100 single-output txs, back-to-back
  Wait for all confirmed at depth >= C_min before recording arm time.

We use a single self-send address for all recipients (spec allows self-sends).
The Transfer gRPC call with multiple PaymentRecipient entries produces ONE tx
with multiple outputs - which is what the batch arm requires.

Note: 'submission throughput' = wall clock from first request to last response.
"Confirmed at depth >= C_min" is measured separately via chain advancement.
"""
import argparse, grpc, json, time, sys
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
    return sf(1, address) + vf(2, amount) + vf(3, fee) + sf(4, "s5") + vf(5, payment_type)

def transfer(ch, recipients_payload):
    """recipients_payload: list of make_recipient() byte payloads."""
    body = b''
    for rp in recipients_payload:
        body += mf(1, rp)
    return ch.unary_unary(
        '/tari.rpc.Wallet/Transfer',
        request_serializer=lambda x: x,
        response_deserializer=lambda x: x,
    )(body, timeout=180)

def parse_transfer_results(resp):
    """Return list of (tx_id, is_success, failure_message)."""
    results = []
    i = 0
    while i < len(resp):
        tag = resp[i]; i += 1
        w = tag & 7
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

def get_address_raw(ch):
    """Return raw bytes from GetAddress, plus base58-encoded address."""
    resp = ch.unary_unary('/tari.rpc.Wallet/GetAddress', request_serializer=lambda x:x, response_deserializer=lambda x:x)(b'', timeout=10)
    i = 0; tag = resp[i]; i += 1
    length = 0; sh = 0
    while True:
        b = resp[i]; i += 1
        length |= (b & 0x7f) << sh
        if not (b & 0x80): break
        sh += 7
    raw = resp[i:i+length]
    import base58
    b0 = base58.b58encode(raw[0:1]).decode()
    b1 = base58.b58encode(raw[1:2]).decode()
    rest = base58.b58encode(raw[2:]).decode()
    return raw, b0 + b1 + rest

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, default=18143)
    ap.add_argument('--M', type=int, default=100)
    ap.add_argument('--K', type=int, default=10)
    ap.add_argument('--amount-ut', type=int, default=1_000_000)  # 1 tXTM per recipient
    ap.add_argument('--output', default='results/S5_mode1_2026-05-30.json')
    args = ap.parse_args()

    ch = grpc.insecure_channel(f'127.0.0.1:{args.port}')

    raw, address = get_address_raw(ch)
    print(f"target address: {address[:32]}... (self-send)", flush=True)
    print(f"M={args.M} K={args.K} amount_per_recipient_ut={args.amount_ut}", flush=True)

    pre_balance = get_balance(ch)
    print(f"pre-test balance: {pre_balance.get(1,0)/1e6:.2f} tXTM available", flush=True)

    out = {
        "scenario": "S5",
        "mode": 1,
        "endpoint": f"http://127.0.0.1:{args.port}",
        "started_at": datetime.now(timezone.utc).isoformat(),
        "config": {"M": args.M, "K": args.K, "amount_per_recipient_ut": args.amount_ut},
        "balance_before_ut": pre_balance.get(1, 0),
    }

    # ── Arm A: Batch ────────────────────────────────────────────────────────
    print(f"\n=== Arm A (batch): {args.M // args.K} txs of {args.K} outputs each ===", flush=True)
    batch_count = args.M // args.K
    batch_tx_results = []
    batch_start = time.perf_counter()
    batch_per_call_times = []
    for i in range(batch_count):
        recipients = [make_recipient(address, args.amount_ut) for _ in range(args.K)]
        t0 = time.perf_counter()
        try:
            resp = transfer(ch, recipients)
            t1 = time.perf_counter()
            parsed = parse_transfer_results(resp)
            batch_per_call_times.append(t1 - t0)
            ok_count = sum(1 for _, ok, _ in parsed if ok)
            print(f"  batch tx {i+1}/{batch_count}: {ok_count}/{len(parsed)} ok, {(t1-t0)*1000:.0f}ms", flush=True)
            batch_tx_results.append({"call_idx": i, "elapsed_secs": t1-t0, "results": parsed})
        except Exception as e:
            t1 = time.perf_counter()
            print(f"  batch tx {i+1}/{batch_count} FAILED: {e}", flush=True)
            batch_tx_results.append({"call_idx": i, "elapsed_secs": t1-t0, "error": str(e)})
            batch_per_call_times.append(t1 - t0)
    batch_wall = time.perf_counter() - batch_start
    print(f"Arm A total: {batch_wall:.2f}s", flush=True)
    out["arm_a_batch"] = {
        "batch_count": batch_count,
        "outputs_per_tx": args.K,
        "wall_secs": batch_wall,
        "per_call_secs": batch_per_call_times,
        "results": batch_tx_results,
    }

    # Pause between arms to let pending settle (otherwise FundsPending bites)
    print("\nwaiting 60s between arms for pending state to clear...", flush=True)
    time.sleep(60)

    # ── Arm B: Individual ───────────────────────────────────────────────────
    print(f"\n=== Arm B (individual): {args.M} single-output txs ===", flush=True)
    indiv_start = time.perf_counter()
    indiv_per_call_times = []
    indiv_tx_results = []
    for i in range(args.M):
        recipient = [make_recipient(address, args.amount_ut)]
        t0 = time.perf_counter()
        try:
            resp = transfer(ch, recipient)
            t1 = time.perf_counter()
            parsed = parse_transfer_results(resp)
            indiv_per_call_times.append(t1 - t0)
            ok = parsed[0][1] if parsed else None
            indiv_tx_results.append({"call_idx": i, "elapsed_secs": t1-t0, "tx_id": parsed[0][0] if parsed else None, "ok": ok, "msg": parsed[0][2] if parsed else None})
            if (i+1) % 10 == 0 or not ok:
                tag = "ok" if ok else f"FAIL: {parsed[0][2] if parsed else 'no result'}"
                print(f"  indiv {i+1}/{args.M}: {(t1-t0)*1000:.0f}ms {tag}", flush=True)
        except Exception as e:
            t1 = time.perf_counter()
            print(f"  indiv {i+1}/{args.M} FAILED: {e}", flush=True)
            indiv_tx_results.append({"call_idx": i, "elapsed_secs": t1-t0, "error": str(e)})
            indiv_per_call_times.append(t1 - t0)
    indiv_wall = time.perf_counter() - indiv_start
    print(f"Arm B total: {indiv_wall:.2f}s", flush=True)
    out["arm_b_individual"] = {
        "tx_count": args.M,
        "wall_secs": indiv_wall,
        "per_call_secs": indiv_per_call_times,
        "results": indiv_tx_results,
    }

    # ── Compute headline ────────────────────────────────────────────────────
    if batch_wall > 0:
        speedup = indiv_wall / batch_wall
    else:
        speedup = 0
    out["headline"] = {
        "throughput_multiplier_individual_over_batch": speedup,
        "batch_avg_call_secs": sum(batch_per_call_times) / len(batch_per_call_times) if batch_per_call_times else 0,
        "individual_avg_call_secs": sum(indiv_per_call_times) / len(indiv_per_call_times) if indiv_per_call_times else 0,
    }
    out["finished_at"] = datetime.now(timezone.utc).isoformat()
    out["balance_after_ut"] = get_balance(ch).get(1, 0)

    with open(args.output, 'w') as f:
        json.dump(out, f, indent=2, default=str)

    print(f"\n=== S5 headline ===", flush=True)
    print(f"Arm A (batch):      {batch_wall:.2f}s for {batch_count} txs × {args.K} outputs", flush=True)
    print(f"Arm B (individual): {indiv_wall:.2f}s for {args.M} txs × 1 output", flush=True)
    print(f"Speedup (B/A):      {speedup:.2f}x", flush=True)
    print(f"\nresults: {args.output}", flush=True)

if __name__ == '__main__':
    main()
