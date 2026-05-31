#!/usr/bin/env python
"""Verify that the S5 txs we counted as 'ok' actually mined.

Same shape as verify_s4_mined.py but for S5. Pulls tx_ids from both arms
(batch + individual), queries GetTransactionInfo, breaks down by status.
"""
import argparse, grpc, json
from collections import Counter

STATUS = {
    0: "TX_COMPLETED", 1: "TX_BROADCAST", 2: "TX_MINED_UNCONFIRMED",
    3: "TX_IMPORTED", 4: "TX_PENDING", 5: "TX_COINBASE",
    6: "TX_MINED_CONFIRMED", 7: "TX_REJECTED",
    8: "TX_ONE_SIDED_UNCONFIRMED", 9: "TX_ONE_SIDED_CONFIRMED",
    10: "TX_QUEUED", 11: "TX_NOT_FOUND",
    12: "TX_COINBASE_UNCONFIRMED", 13: "TX_COINBASE_CONFIRMED",
    14: "TX_COINBASE_NOT_IN_BLOCK_CHAIN",
}
TERMINAL_OK = {2, 6, 9, 13}
TERMINAL_FAIL = {7, 11, 14}

def varint(n):
    out = b''
    while True:
        b = n & 0x7f; n >>= 7
        if n: out += bytes([b | 0x80])
        else: out += bytes([b]); return out
def vfield(t, n): return varint((t<<3)|0) + varint(n)

def get_tx_status(ch, tx_id):
    body = vfield(1, tx_id)
    try:
        resp = ch.unary_unary('/tari.rpc.Wallet/GetTransactionInfo',
            request_serializer=lambda x: x, response_deserializer=lambda x: x)(body, timeout=10)
    except Exception:
        return None, 'NO_RESPONSE'
    i = 0
    while i < len(resp):
        tag = resp[i]; i += 1
        if tag != 0x0a: continue
        length = 0; sh = 0
        while True:
            b = resp[i]; i += 1
            length |= (b & 0x7f) << sh
            if not (b & 0x80): break
            sh += 7
        ti = resp[i:i+length]; i += length
        j = 0
        while j < len(ti):
            tb = ti[j]; j += 1
            fn = tb >> 3; w = tb & 7
            if w == 0:
                v = 0; sh = 0
                while True:
                    b = ti[j]; j += 1
                    v |= (b & 0x7f) << sh
                    if not (b & 0x80): break
                    sh += 7
                if fn == 4: return v, STATUS.get(v, f'UNK({v})')
            elif w == 2:
                sl = 0; sh = 0
                while True:
                    b = ti[j]; j += 1
                    sl |= (b & 0x7f) << sh
                    if not (b & 0x80): break
                    sh += 7
                j += sl
            elif w == 1: j += 8
            elif w == 5: j += 4
            else: break
        return None, 'NO_STATUS'
    return None, 'EMPTY'

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, default=18243)
    ap.add_argument('--input', default='results/S5_mode1_fresh_2026-05-30.json')
    ap.add_argument('--output', default='results/S5_mined_verification.json')
    args = ap.parse_args()

    with open(args.input) as f:
        s5 = json.load(f)

    ch = grpc.insecure_channel(f'127.0.0.1:{args.port}')
    out = {"verification_for": args.input, "endpoint": f"http://127.0.0.1:{args.port}"}

    # Arm A — collect first tx_id per batch call
    arm_a_tx_ids = []
    for entry in s5.get('arm_a_batch', {}).get('results', []):
        for tx_id, ok, _ in (entry.get('results') or []):
            if tx_id and ok:
                arm_a_tx_ids.append(tx_id); break

    # Arm B
    arm_b_tx_ids = []
    for entry in s5.get('arm_b_individual', {}).get('results', []):
        if entry.get('tx_id') and entry.get('ok'):
            arm_b_tx_ids.append(entry['tx_id'])

    for arm_name, tx_ids in (("arm_a_batch", arm_a_tx_ids), ("arm_b_individual", arm_b_tx_ids)):
        breakdown = Counter()
        for tx_id in tx_ids:
            _, name = get_tx_status(ch, tx_id)
            breakdown[name] += 1
        ok = sum(c for n, c in breakdown.items() if n in {STATUS[i] for i in TERMINAL_OK})
        fail = sum(c for n, c in breakdown.items() if n in {STATUS[i] for i in TERMINAL_FAIL})
        out[arm_name] = {
            "tx_ids_checked": len(tx_ids),
            "verified_mined": ok,
            "terminal_failure": fail,
            "status_breakdown": dict(breakdown),
        }
        print(f"{arm_name}: {len(tx_ids)} tx_ids checked, {ok} mined, {fail} failed")
        if breakdown:
            print(f"  breakdown: {dict(breakdown)}")

    with open(args.output, 'w') as f:
        json.dump(out, f, indent=2)
    print(f"\nwritten to {args.output}")

if __name__ == '__main__':
    main()
