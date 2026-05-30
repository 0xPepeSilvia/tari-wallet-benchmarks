# Wallet benchmark harness — first end-to-end run

**Date**: 2026-05-30
**Network**: Esmeralda testnet
**Operator**: callum
**Chain tip at run start**: 670,375
**Mining wallet balance at start**: 209,023 tXTM (2.8x the 75k target)

## Scenarios completed

| Scenario | Mode 1 (console_wallet) | Mode 2 (minotari-cli) | Mode 3 (payment_processor) |
|---|---|---|---|
| Smoke test | ok | ok (daemon HTTP API) | not attempted |
| **B0** | **8.18 s, 670,375 blocks, ~82,000 blocks/s** | blocked - Finding #7 | not attempted |
| **S0** | **150 s broadcast → confirmed, 30k tXTM clean** | < 10 s on daemon startup after recreate (Finding #9) | not attempted |
| **S1** | **115/127 (91%)**: full doubling 63/63, fanout 52/64 | blocked - Findings #10, #11 | not attempted |
| **S4** | **clean ramp** N=8..128 all 100%, 12.9-16.4 tx/s | not attempted (Finding #10) | not attempted |
| **S5** | **clean batch vs individual, 1.07x multiplier** | not attempted | not attempted |
| S2/S6 | not run (requires wallet-restart-with-birthday-rewrite) | blocked - Finding #7 | not attempted |
| S3/S7 | not run (same as S2) | reachable via daemon initial scan | not attempted |

Mode 3 binary built failed at the sqlx compile-time query check stage (30 errors) despite DB creation + migrations completing. Mode 3 deferred for a follow-up session.

## Headline measurements

### Mode 1 B0 — genesis scan, empty wallet

670,375 blocks / 8.178 s = **82,000 blocks/s**
Floor cost: block walk + view-key check with no UTXO writes. Dominated by base-node-side header retrieval over local gRPC.

### Mode 1 S1 — UTXO build-up (doubling phase 100%, fanout 81%)

| Round | Txs | Per-tx construct (avg) | Wall clock |
|---|---|---|---|
| 1 doubling | 1 | 36 ms | 150 s |
| 2 doubling | 2 | 196 ms | 120 s |
| 3 doubling | 4 | 34 ms | 180 s |
| 4 doubling | 8 | 33 ms | 120 s |
| 5 doubling | 16 | 35 ms | 181 s |
| 6 doubling | 32 | 39 ms | 181 s |
| fanout | 52 of 64 | ~80 ms | aborted |

Doubling total: 63 txs in 933 s (~15.5 min). Per-tx construct = 41 ms average. Wall clock dominated by 4-block mine wait between rounds.

### Mode 1 S4 — concurrent construction throughput

Wallet sustains 13-16 tx/s across the entire N=8..128 ramp with a deep UTXO pool:

| N | Successes | tx/s | Max serialisation gap |
|---|---|---|---|
| 8 | 8/8 | 16.4 | 78 ms |
| 16 | 16/16 | 16.2 | 78 ms |
| 32 | 32/32 | 15.6 | 81 ms |
| 64 | 64/64 | 14.0 | 93 ms |
| 128 | 128/128 | 12.9 | 245 ms |

~20% throughput drop from N=8 to N=128, consistent with expected per-call locking overhead. No structural ceiling found within the spec range.

The earlier "N=32 cliff" observed on the mining wallet (Finding #12) was UTXO pool exhaustion, not a wallet limit. With proper S1 build-up before S4, this collapses.

### Mode 1 S5 — payment processor batch vs individual (THE HEADLINE)

| Arm | Tx count | Wall clock | Per-tx | Per-recipient |
|---|---|---|---|---|
| A - batch (10 recipients/tx) | 10 | 8.67 s | 867 ms | **88 ms** |
| B - individual (1 recipient/tx) | 100 | 9.30 s | 93 ms | **93 ms** |

**Throughput multiplier (B/A): 1.07x**

Batching saves only ~5 ms per recipient (the gRPC round-trip overhead). Per-output construction (commitment, range proof, output features) is the dominant cost and scales linearly with output count. The 5-10x speedup typical of payment-processor batch claims does NOT materialise on Mode 1.

The on-chain footprint advantage of batching (fewer transactions, fewer signatures, lower aggregate fee) remains valid. Wall-clock throughput is barely affected because the wallet's per-output work is the same either way.

## 15 wallet pains surfaced

All recorded in `results/wallet_pain_findings.md` with reproduction steps. Per the bounty principle ("harness measures, does not engineer around wallet pain"), each is treated as a result.

| # | Finding | Severity |
|---|---|---|
| 1 | `tari.rpc.Wallet/RescanWallet(from_height=0)` rescans only ~5000 blocks, not from genesis | blocks S2/S6 |
| 2 | `CoinSplit` → `FundsPending` on serial spends from outputs of unmined parent tx | makes S1 harder than spec implies |
| 3 | `GetState` proto shape differs from older docs | onboarding friction |
| 4 | `has_done_initial_validation` is the modern readiness signal, not `is_bootstrapped` | onboarding friction |
| 5 | `CoinSplit` → `NotEnoughFunds` when amount > smallest UTXO (no auto-aggregate) | sizing care required |
| 6 | `CoinSplit` → `FundsPending` mid-round when spendable pool exhausts | structural constraint on S1 |
| 7 | Mode 2 has no API to do a genesis rescan | blocks Mode 2 B0/S2/S6 |
| 8 | Mode 2 daemon enters silent-zombie state after `re-scan` | recovery requires DB delete |
| 9 | Mode 2 seed words encode birthday and persist through DB recreate | partial workaround for #7 |
| 10 | Mode 2 `create-unsigned-transaction` locks entire balance for 24h | blocks Mode 2 S1/S4/S5 |
| 11 | minotari emits unsigned tx v4.0.0; console_wallet signer expects v5.0.0 | Mode 2 sign+broadcast pipeline broken at current main |
| 12 | "N=32 cliff" observed on shallow UTXO pool (initial observation) | superseded by #14 |
| 13 | Post-concurrent-stress wallet rejects new Transfer calls for >60 s | spec S4→S5 sequence needs cool-down |
| 14 | The "N=32 cliff" was UTXO-pool exhaustion, not a wallet ceiling | rewrites #12's interpretation |
| 15 | S5 batch speedup is 1.07x, not the 5-10x typical of payment-processor claims | headline finding |

## Reproducibility

- Source: `C:/Projects/tari-wallet-benchmarks` (13 commits today on master)
- Mode 1 wallet: `C:/Tari-bench-mode1/` (preserved; ~470 UTXOs)
- Mode 2 wallet: `C:/Tari-bench-mode2/` (preserved; seed words in `S0_mode2_2026-05-30.json`)
- Mining wallet: `C:/Tari-esmeralda/` (preserved)
- Base node: `http://127.0.0.1:18142` (gRPC), `http://127.0.0.1:9005` (HTTP/json_rpc)

Per-scenario result JSONs:
- `results/B0_mode1_2026-05-30.json`
- `results/S0_mode1_2026-05-30.json`
- `results/S1_mode1_v4_2026-05-30.json` (+ `S1_mode1_summary_2026-05-30.md`)
- `results/S4_mode1_fresh_2026-05-30.json` (clean), `S4_mode1_mining_2026-05-30.json` (initial)
- `results/S5_mode1_fresh_2026-05-30.json` (clean), `S5_mode1_mining_2026-05-30.json` (saturated)
- `results/B0_mode2_2026-05-30.json` (blocked), `S0_mode2_2026-05-30.json`
- `results/wallet_pain_findings.md`

## What this run does NOT cover

- **Mode 1 S2/S3/S6/S7**: rescan scenarios require either a wallet-restart-with-birthday-0 sequence (needs seed-word capture and a fresh-base-path recovery launch) or a wallet API to set birthday. We have an unintended B0-like recovery measurement (24,344 blocks/s) captured during a Mode 2 sign attempt - but on an empty wallet, so that's effectively B0 at a different binary path, not S2.
- **Mode 2 B0/S1/S4/S5**: blocked by Findings #7, #10, #11. Either the bounty PR #99 library path (`tari_transaction_components::offline_signing::sign_locked_transaction` as a cargo dep) or a fix to the create-unsigned/sign-one-sided version mismatch is required.
- **Mode 3 entirely**: minotari_payment_processor binary did not compile at current main (30 sqlx errors after DB creation + migration). Investigation deferred.
- **Per-tx fees in S5**: Transfer responses do not return fee; would need a follow-up `GetTransactionInfo` poll per returned tx_id.

## Recommended next session

1. Drive `console_wallet --recovery --seed-words "..."` cycle on the existing Mode 1 wallet to measure S2 (recovery time with the ~470-UTXO pool we just built).
2. Capture per-tx fees for S5 via a post-arm `GetTransactionInfo` poll loop.
3. Investigate minotari-cli library-path signing (link tari_transaction_components, call sign_locked_transaction) to unblock Mode 2 S1/S4/S5.
4. Fix the create-unsigned (v4) / sign-one-sided (v5) version mismatch in minotari-cli OR document the required upstream change.
5. Diagnose payment_processor's sqlx compile errors (likely needs a specific sqlx-cli version or `SQLX_OFFLINE=true` with pre-generated `.sqlx` cache).
