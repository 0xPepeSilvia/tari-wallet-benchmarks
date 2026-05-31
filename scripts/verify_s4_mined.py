#!/usr/bin/env python
"""Verify that the S4 txs we counted as 'ok' actually mined.

Reads results/S4_mode1_fresh_2026-05-30.json, pulls every tx_id from
per_worker_results where outcome == "ok", queries GetTransactionInfo
via gRPC, and reports the breakdown by TransactionStatus.

Catches the failure mode where CoinSplit returns success (mempool
accepted) but the tx is later rejected by the chain — our throughput
number would over-count broadcasts in that case.
"""
import argparse, grpc, json, sys
from collections import Counter

# TransactionStatus enum (exact values from
# tari/applications/minotari_app_grpc/proto/wallet.proto:2165)
STATUS = {
    0: "TX_COMPLETED",
    1: "TX_BROADCAST",
    2: "TX_MINED_UNCONFIRMED",
    3: "TX_IMPORTED",
    4: "TX_PENDING",
    5: "TX_COINBASE",
    6: "TX_MINED_CONFIRMED",
    7: "TX_REJECTED",
    8: "TX_ONE_SIDED_UNCONFIRMED",
    9: "TX_ONE_SIDED_CONFIRMED",
    10: "TX_QUEUED",
    11: "TX_NOT_FOUND",
    12: "TX_COINBASE_UNCONFIRMED",
    13: "TX_COINBASE_CONFIRMED",
    14: "TX_COINBASE_NOT_IN_BLOCK_CHAIN",
}

TERMINAL_OK = {2, 6, 9, 13}    # mined-unconfirmed, mined-confirmed, one-sided-confirmed, coinbase-confirmed
TERMINAL_FAIL = {7, 11, 14}    # rejected, not found, coinbase not in chain
PENDING = {0, 1, 3, 4, 5, 8, 10, 12}  # everything else still propagating / queued / faux

def varint(n):
    out = b''
    while True:
        b = n & 0x7f; n >>= 7
        if n: out += bytes([b | 0x80])
        else: out += bytes([b]); return out
def vfield(t, n): return varint((t<<3)|0) + varint(n)

def get_tx_status(ch, tx_id):
    """Returns (status_int, status_name) or (None, 'NO_RESPONSE')."""
    body = vfield(1, tx_id)
    try:
        resp = ch.unary_unary(
            '/tari.rpc.Wallet/GetTransactionInfo',
            request_serializer=lambda x: x,
            response_deserializer=lambda x: x,
        )(body, timeout=10)
    except Exception:
        return None, 'NO_RESPONSE'

    # Parse first TransactionInfo, find field 4 (status, varint enum)
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
                if fn == 4:
                    return v, STATUS.get(v, f'UNKNOWN({v})')
            elif w == 2:
                sl = 0; sh = 0
                while True:
                    b = ti[j]; j += 1
                    sl |= (b & 0x7f) << sh
                    if not (b & 0x80): break
                    sh += 7
                j += sl
            elif w == 1:
                j += 8
            elif w == 5:
                j += 4
            else:
                break
        return None, 'NO_STATUS_FIELD'
    return None, 'EMPTY_RESPONSE'

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, default=18243)
    ap.add_argument('--input', default='results/S4_mode1_fresh_2026-05-30.json')
    ap.add_argument('--output', default='results/S4_mined_verification.json')
    args = ap.parse_args()

    with open(args.input) as f:
        s4 = json.load(f)

    ch = grpc.insecure_channel(f'127.0.0.1:{args.port}')

    out = {
        "verification_for": args.input,
        "endpoint": f"http://127.0.0.1:{args.port}",
        "per_level": [],
    }

    overall_total = 0
    overall_terminal_ok = 0
    overall_terminal_fail = 0
    overall_pending = 0
    overall_not_found = 0

    for level in s4.get('levels', []):
        N = level['N']
        tx_ids = []
        for w in level.get('per_worker_results', []):
            if w.get('outcome') == 'ok' and w.get('tx_id') is not None:
                tx_ids.append(w['tx_id'])

        statuses = []
        breakdown = Counter()
        for tx_id in tx_ids:
            status_int, status_name = get_tx_status(ch, tx_id)
            statuses.append({"tx_id": tx_id, "status": status_name})
            breakdown[status_name] += 1

        ok = sum(1 for s in statuses if s['status'] in {STATUS[i] for i in TERMINAL_OK})
        fail = sum(1 for s in statuses if s['status'] in {STATUS[i] for i in TERMINAL_FAIL})
        pending = sum(1 for s in statuses if s['status'] in {STATUS[i] for i in PENDING})
        not_found = sum(1 for s in statuses if s['status'] in ('NO_RESPONSE', 'NO_STATUS_FIELD', 'EMPTY_RESPONSE', STATUS[8]))

        out["per_level"].append({
            "N": N,
            "claimed_ok": level['txs_constructed_ok'],
            "tx_ids_checked": len(tx_ids),
            "actually_mined_confirmed_or_one_sided_confirmed": ok,
            "terminal_failure": fail,
            "still_pending": pending,
            "not_found_or_no_response": not_found,
            "status_breakdown": dict(breakdown),
        })

        overall_total += len(tx_ids)
        overall_terminal_ok += ok
        overall_terminal_fail += fail
        overall_pending += pending
        overall_not_found += not_found

        print(f"N={N}: claimed_ok={level['txs_constructed_ok']}, "
              f"verified_mined={ok}, failed={fail}, pending={pending}, not_found={not_found}",
              flush=True)
        if breakdown:
            print(f"  breakdown: {dict(breakdown)}", flush=True)

    out["overall"] = {
        "total_tx_ids_checked": overall_total,
        "mined_or_confirmed": overall_terminal_ok,
        "terminal_failure": overall_terminal_fail,
        "still_pending": overall_pending,
        "not_found_or_no_response": overall_not_found,
        "fraction_verified_mined": (overall_terminal_ok / overall_total) if overall_total > 0 else 0,
    }

    print(f"\n=== Overall: {overall_terminal_ok}/{overall_total} txs verified mined "
          f"({out['overall']['fraction_verified_mined']*100:.1f}%) ===")
    print(f"  pending: {overall_pending}, failed: {overall_terminal_fail}, not_found: {overall_not_found}")

    with open(args.output, 'w') as f:
        json.dump(out, f, indent=2)
    print(f"\nwritten to {args.output}")

if __name__ == '__main__':
    main()
