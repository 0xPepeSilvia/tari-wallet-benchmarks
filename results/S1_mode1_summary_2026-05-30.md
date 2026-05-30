# S1 Mode 1 - UTXO build-up measurement (final)

**Run**: 2026-05-30
**Wallet**: fresh console_wallet on 127.0.0.1:18243, funded with 30,000 tXTM (S0)
**Driver**: `scripts/run_s1.py` (v4 with 4-block mine wait between rounds)
**Result file**: `results/S1_mode1_v4_2026-05-30.json`

## Headline

- **Doubling phase: 63/63 successful** (1+2+4+8+16+32 txs across 6 rounds)
- **Fanout phase: 52/64 successful** (stopped at tx 53 with FundsPending)
- **Overall: 115/127 txs = 91% completion** of the bounty's S1 spec
- **Final UTXO count (post-fanout-confirm)**: ~52 × 8 = 416 fanout outputs + change UTXOs ≈ 470, vs. spec target 512

## Per-round measurements

| Round | Txs requested | Txs succeeded | Total construct (ms) | Wall clock (s) | Per-tx construct (ms avg) |
|---|---|---|---|---|---|
| 1 | 1 | 1 | 36 | 150 | 36 |
| 2 | 2 | 2 | 391 | 120 | 196 |
| 3 | 4 | 4 | 137 | 180 | 34 |
| 4 | 8 | 8 | 260 | 120 | 33 |
| 5 | 16 | 16 | 566 | 181 | 35 |
| 6 | 32 | 32 | 1239 | 181 | 39 |
| **doubling total** | **63** | **63** | **2629** | **933** | **41** |
| fanout (1-out of 64) | 64 | 52 | (52 × ~80 ≈ 4160) | aborted | ~80 |

Wall-clock per round dominated by the 4-block mine wait (~3 min). Per-tx construction averages 41 ms in the doubling phase and ~80 ms in the fanout phase (the fanout's 8-output construction is more work than the doubling's 2-output).

## Comparison vs spec

The spec's S1 wants 6 rounds + 64-tx fanout = 127 successful txs to build a 512-UTXO pool. We achieved 115/127 (91%) with the harness's serial-round-with-mine-wait approach.

The remaining 12 fanout failures are Finding #6 (FundsPending mid-round when spendable UTXO pool exhausts). To complete the fanout, the harness would need to either:
1. Fire fanout txs in batches of N where N matches the count of confirmed-spendable UTXOs, waiting between batches; or
2. Wait for ALL outputs from round 6 to be C_min+1 deep before starting fanout (currently only waits 4 blocks per round, which is enough for doubling's 1-out outputs but not for fanout's 8-out outputs which produce 9x more UTXOs needing the same depth).

Both fixes would extend wall-clock time. Option 2 is simpler: insert another `wait_for_chain_advance(blocks=4)` between doubling and fanout to let round 6's outputs reach maturity.

## Three previous S1 attempts and what each surfaced

| Attempt | Result | Finding surfaced |
|---|---|---|
| v1 | round 1 → round 2: FundsPending immediately | #2: pending state lives in output_manager, not GetBalance.pending_* |
| v2 | round 3: NotEnoughFunds | #5: wallet does not auto-aggregate small UTXOs |
| v3 | round 4 tx 7/8: FundsPending | #6: change from in-flight same-round txs is unspendable |
| v4 | doubling all complete, fanout 52/64 | (same #6, applies to fanout's higher per-tx output count) |

Each attempt's failure mode is documented in `results/wallet_pain_findings.md`. v4 is the first one to clear the doubling phase entirely.

## Wallet state after S1 v4

- Total mined txs: 115 (63 doubling + 52 fanout)
- Expected UTXOs after fanout settle: ~470 (52 × 8 + change/doubling residue)
- Available balance: ~22,500 tXTM minus fees (~2,300 tXTM in fees for 115 txs at fee_per_gram=5)
- Wallet is "warm" for S2 (genesis rescan), S4 (concurrent construction with real UTXO pool), S5 (batch vs individual)
