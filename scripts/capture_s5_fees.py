#!/usr/bin/env python
"""Capture per-tx fees for S5 by polling GetTransactionInfo on each tx_id.

Reads results/S5_mode1_fresh_*.json, extracts the tx_ids from both arms,
queries GetTransactionInfo for each, and writes a fee-augmented JSON.
"""
import json, sys, grpc, time, argparse

def varint(n):
    out = b''
    while True:
        b = n & 0x7f; n >>= 7
        if n: out += bytes([b | 0x80])
        else: out += bytes([b]); return out
def vf(t,n): return varint((t<<3)|0) + varint(n)

def get_tx_info(ch, tx_id):
    """GetTransactionInfoRequest{repeated uint64 transaction_ids=1}.
    Returns GetTransactionInfoResponse{repeated TransactionInfo transactions=1}
    TransactionInfo fields we want: 6=amount, 7=fee, 4=status (enum).
    """
    body = vf(1, tx_id)
    try:
        resp = ch.unary_unary(
            '/tari.rpc.Wallet/GetTransactionInfo',
            request_serializer=lambda x: x,
            response_deserializer=lambda x: x,
        )(body, timeout=10)
    except Exception as e:
        return {"error": str(e)}

    # Parse - resp is GetTransactionInfoResponse with field 1 = repeated TransactionInfo (wire 2)
    i = 0
    while i < len(resp):
        tag = resp[i]; i += 1
        if tag != 0x0a:  # field 1, wire 2
            continue
        length = 0; sh = 0
        while True:
            b = resp[i]; i += 1
            length |= (b & 0x7f) << sh
            if not (b & 0x80): break
            sh += 7
        ti = resp[i:i+length]; i += length
        # Parse TransactionInfo
        info = {}
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
                info[fn] = v
            elif w == 2:
                sl = 0; sh = 0
                while True:
                    b = ti[j]; j += 1
                    sl |= (b & 0x7f) << sh
                    if not (b & 0x80): break
                    sh += 7
                pv = ti[j:j+sl]; j += sl
                if fn in (2, 3, 9, 11, 12):
                    try: info[fn] = pv.decode('utf-8', errors='replace')
                    except: info[fn] = pv.hex()
                else:
                    info[fn] = pv.hex()
            elif w == 1:
                j += 8  # 64-bit fixed
            elif w == 5:
                j += 4  # 32-bit fixed
            else:
                # Unknown wire type, abort cleanly
                break
        return {
            "tx_id": info.get(1),
            "status": info.get(4),  # TransactionStatus enum
            "direction": info.get(5),
            "amount": info.get(6),
            "fee": info.get(7),
            "is_cancelled": info.get(8),
            "timestamp": info.get(10),
            "message": info.get(11, ""),
        }
    return {"error": "no transaction in response"}

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, default=18243)
    ap.add_argument('--input', default='results/S5_mode1_fresh_2026-05-30.json')
    ap.add_argument('--output', default='results/S5_mode1_fresh_2026-05-30_with_fees.json')
    args = ap.parse_args()

    with open(args.input) as f:
        d = json.load(f)

    ch = grpc.insecure_channel(f'127.0.0.1:{args.port}')

    # Collect tx_ids per arm
    arm_a_tx_ids = []
    for entry in d.get("arm_a_batch", {}).get("results", []):
        for tx_id, ok, _ in (entry.get("results") or []):
            if tx_id and ok:
                arm_a_tx_ids.append(tx_id)
                break  # one tx_id per batch call (the batched tx)

    arm_b_tx_ids = []
    for entry in d.get("arm_b_individual", {}).get("results", []):
        tx_id = entry.get("tx_id")
        if tx_id and entry.get("ok"):
            arm_b_tx_ids.append(tx_id)

    print(f"Arm A tx_ids: {len(arm_a_tx_ids)}", flush=True)
    print(f"Arm B tx_ids: {len(arm_b_tx_ids)}", flush=True)

    def collect(ids, label):
        out = []
        total_fee = 0; total_amt = 0
        for i, tx_id in enumerate(ids):
            info = get_tx_info(ch, tx_id)
            out.append(info)
            if isinstance(info.get("fee"), int):
                total_fee += info["fee"]
            if isinstance(info.get("amount"), int):
                total_amt += info["amount"]
            if (i+1) % 10 == 0:
                print(f"  {label} {i+1}/{len(ids)}", flush=True)
        return out, total_fee, total_amt

    arm_a_info, a_fee, a_amt = collect(arm_a_tx_ids, "arm A")
    arm_b_info, b_fee, b_amt = collect(arm_b_tx_ids, "arm B")

    # Spec wants: per arm = total fees, fee-per-recipient
    M = d.get("config", {}).get("M", 100)
    K = d.get("config", {}).get("K", 10)

    fees = {
        "arm_a_batch": {
            "tx_count": len(arm_a_info),
            "total_fee_ut": a_fee,
            "total_amount_ut": a_amt,
            "fee_per_recipient_ut": a_fee / M if M else 0,
            "fee_per_tx_ut": a_fee / len(arm_a_info) if arm_a_info else 0,
            "per_tx_info": arm_a_info,
        },
        "arm_b_individual": {
            "tx_count": len(arm_b_info),
            "total_fee_ut": b_fee,
            "total_amount_ut": b_amt,
            "fee_per_recipient_ut": b_fee / M if M else 0,
            "fee_per_tx_ut": b_fee / len(arm_b_info) if arm_b_info else 0,
            "per_tx_info": arm_b_info,
        },
        "fee_ratio_individual_over_batch": b_fee / a_fee if a_fee else 0,
    }

    d["fees"] = fees

    print(f"\n=== S5 fees ===", flush=True)
    print(f"Arm A batch:      total {a_fee} µT ({a_fee/1e6:.4f} tXTM), {a_fee/M:.0f} µT/recipient", flush=True)
    print(f"Arm B individual: total {b_fee} µT ({b_fee/1e6:.4f} tXTM), {b_fee/M:.0f} µT/recipient", flush=True)
    print(f"Fee ratio (B/A):  {b_fee/a_fee:.2f}x" if a_fee else "(A had 0 fees, ratio undefined)", flush=True)

    with open(args.output, 'w') as f:
        json.dump(d, f, indent=2, default=str)
    print(f"\nwritten to {args.output}", flush=True)

if __name__ == '__main__':
    main()
