# Wallet pains — Mode 1 (`minotari_console_wallet`), Mode 2 (`minotari-cli`), Mode 3 (`minotari_payment_processor`)

Run date: 2026-05-30 → 2026-06-01
Network: Esmeralda
Methodology: harness drives the wallets via their advertised surfaces (gRPC for Mode 1, HTTP daemon API and CLI subprocess for Mode 2, HTTP API for Mode 3). Per the bounty principle, friction is surfaced as a result rather than engineered around.

This file is the consolidated, sharp-item version. Earlier drafts had 19 entries; some were corrections of my own initial mistakes, some were restatements of the same root cause at different layers. This rewrite is **8 items**, each tied directly to an upstream change a Tari maintainer can act on, with reproduction context.

The full first-pass log of every observation, including the ones folded below, lives in the commit history of this repo (`git log --grep=Finding`).

---

## 1. Mode 2 sign pipeline is incompatible at current main (v4 vs v5 unsigned-tx format)

**The blocker for Mode 2 and Mode 3 end-to-end.**

`minotari create-unsigned-transaction` (minotari-cli current main) emits unsigned tx JSON with `"version": "4.0.0"`.
`minotari_console_wallet sign-one-sided-transaction` (tari current main) rejects it with `Error serializing transaction: Unsupported version. Expected '5.0.0', got '4.0.0'`.

Production-scale reproduction is documented in `results/S5_mode3_2026-05-30.json`: after wiring up `minotari_payment_processor` against a live `minotari-cli` daemon + a recovered console_wallet for signing, the PP pipeline progresses from `BATCHED` → `AwaitingSignature` → `SigningInProgress`, gets a valid v4.0.0 unsigned tx from the PR, calls `console_wallet sign-one-sided-transaction` as a subprocess, gets exit 107 with the version-mismatch stderr above. PP reverts the batch to `AwaitingSignature` and retries every 10 seconds. Infinite loop.

**Affects**: Mode 2 send-side (S0/S1/S4/S5), Mode 3 entirely (every payment that needs signing).
**Fix needed**: pick one — either `minotari create-unsigned-transaction` bumps its output to v5.0.0, or `console_wallet sign-one-sided-transaction` learns to accept v4.0.0. The two tools that should compose to form Mode 2 and Mode 3 are not currently composable.

---

## 2. Mode 2 has no API to do a genesis rescan

`minotari-cli` exposes `re-scan --rescan-from-height 0` and a daemon HTTP API. Neither path actually resets scan to genesis on an existing wallet:

- `re-scan --rescan-from-height 0` rolls back DB state (BlockRolledBack events, soft-deletes outputs/inputs/scanned-tip-blocks down to height 0) and prints `Re-scan complete event_count=0`. But the wallet's `last_scanned_height` value in the DB is not reset. Daemon resumes from the previous tip-adjacent value on next start.
- `minotari create` has no `--birthday` flag. A fresh wallet's birthday is the current chain tip at creation time.

SWvheerden's earlier guidance to roadhero ("change the seed words so the encoded birthday reflects 0") describes the intended path. No CLI surface implements it.

**Affects**: Mode 2 B0/S2/S6 entirely.
**Fix needed**: either a `--birthday N` flag on `minotari create`, or a daemon endpoint `POST /accounts/{name}/set_birthday` that rewinds `last_scanned_height` and triggers a full re-scan.

---

## 3. Mode 2 `create-unsigned-transaction` locks the entire balance for 24 hours on first call

`minotari create-unsigned-transaction`'s default `--seconds-to-lock` is 86,400. After a single call on a wallet with one UTXO, every subsequent call within 24 hours returns `Failed to lock funds: Funds are pending. Available: 0 µT, Pending: <full balance>, Required: X`.

This is independent of #1: even with the version mismatch fixed, a Mode 2 wallet cannot construct round 2 of S1 until round 1 mines + the change UTXO becomes spendable. Combined with #1, the wallet is stuck after the first construct call until either the lock expires or the wallet DB is wiped + recreated.

**Affects**: Mode 2 S1 doubling (each round assumes available change UTXOs), Mode 2 S4 (concurrent construction), Mode 2 S5 (back-to-back batches).
**Fix needed**: the default lock window is too long for any realistic test. Either reduce the default to minutes, or surface the lock state clearly in error messages so operators know what's holding the balance.

---

## 4. `minotari_console_wallet` has no scriptable seed-word export

The wallet creates a 24-word seed on first launch and stores it encrypted in `console_wallet.db`. The TUI exposes a menu to reveal it interactively. Non-interactive mode has no equivalent command.

Available subcommands inspected: `export-view-key-and-spend-key` (emits keys, not seed words), `whois <PUBLIC_KEY>` (public-key lookup), `show-pay-ref` (payment ID lookup). `--seed-words-file-name <PATH>` writes the seed during wallet CREATION, not for an existing wallet.

**Affects**: Reproducibility of S2/S3/S6/S7 on Mode 1. To recover a wallet that holds the S1 UTXO pool into a fresh base path (the spec's S2), an operator needs the seed words. Without scriptable export, the operator must drive the TUI interactively. Our workaround (see `results/S2_mode1_2026-06-01.json`): build the UTXO pool by sending fragments to a wallet whose seed we already know (Mode 2's daemon wallet, where we extracted the seed at creation time via the minotari-cli daemon HTTP API).
**Fix needed**: add `minotari_console_wallet show-seed-words --password ... --base-path ...` (mirrors the existing `minotari-cli show-seed-words`).

---

## 5. `RescanWallet(from_height=0)` over gRPC does NOT scan from genesis

`tari.rpc.Wallet/RescanWallet` accepts a `from_height` field. Calling with `from_height=0` produces a visible drop in `GetState.scanned_height` of only ~5,000 blocks before climbing back to tip. No full chain rescan.

Reproduction in `results/rust_harness_endtoend_s0_mode1.json` and earlier `rescan_bench_result.json` notes: scanned_height dropped from 670,316 to 665,390 then climbed back in ~10 s. Floor was approximately tip − 5,000 blocks regardless of the requested from_height.

**Affects**: Any harness that uses `RescanWallet` to implement S2/S3/S6/S7 will silently produce a ~5,000-block measurement and call it a 670,000-block scan. The Rust runner in this repo did exactly that on first attempt; we caught it because the throughput number (133,821 blocks/sec) was obviously wrong.
**Fix needed**: either honor `from_height`, or remove the field from the RPC (and document `console_wallet --recovery --seed-words "..." --base-path <fresh>` as the only supported genesis-rescan path).

---

## 6. `OutputManagerError(FundsPending)` lives in the output_manager, not in `GetBalance` accounting

Spending a UTXO created by an unmined tx returns `OutputManagerError(FundsPending)` even when `GetBalance.pending_in/out` are 0. The pending state lives in the output_manager's lock table, not the balance accounting layer.

This shows up in multiple shapes:
- **Round-to-round on S1**: after broadcasting round N's CoinSplits, round N+1's first attempt errors with FundsPending even though balance fields show 0 pending. Mitigation: wait for chain to advance ≥ 4 blocks between rounds (we observed `C_min+1 = 2` was not enough; 4 was empirically reliable).
- **Mid-round on S1**: round 6 needs 32 CoinSplits but the wallet had 7 confirmed-spendable UTXOs going in; calls 8/8 fail because the change from same-round earlier calls is unspendable until those parent txs mine.
- **Post-S4 saturation**: after firing 248 concurrent CoinSplit calls (21 succeed), 100% of subsequent Transfer calls fail with FundsPending for at least 60 s, regardless of remaining confirmed UTXOs.

A naive harness that polls `GetBalance.pending_*` for "ready" signals will not see the pending state and will spin on FundsPending until the operator notices.

**Affects**: S1 mid-round retry logic, S4 → S5 sequencing, any agentic harness reading balance fields to gate spends.
**Fix needed**: either expose `OutputManager.locked_outputs_count` and `OutputManager.pending_unmined_outputs_count` via `GetState`, or include the same surface in the error message so the harness can wait on the right thing.

---

## 7. S5 batch advantage is fee-only, not throughput, and self-sends are the friendliest case

Three S5 runs against the same Mode 1 wallet, varying recipient choice:

| Run | Arm A batch (10×10 outputs) | Arm B individual (100×1) | Wall ratio B/A | Per-recipient batch | Per-recipient individual |
|---|---|---|---|---|---|
| Self-sends | 8.67 s | 9.30 s | 1.07× | 88 ms | 93 ms |
| 100 distinct recipients | **27.25 s** | **18.46 s** | **0.68×** | **272 ms** | **185 ms** |

Distinct recipients are uniformly more expensive (each one needs a fresh stealth address derivation). The cost grows MORE inside a batch tx than across individual txs — batch construction within one tx serialises the derivations, while individual gRPC calls overlap better.

Fee ratio is the inverse story:

| Run | Arm A batch total | Arm B individual total | Fee ratio B/A |
|---|---|---|---|
| Self-sends | 7,400 µT | 78,775 µT | 10.65× |
| Distinct recipients | 7,400 µT | 71,045 µT | 9.60× |

For payment-processor workloads:
- **Wall clock**: individual wins by 47% with distinct recipients
- **On-chain fees**: batch wins by ~10× either way

The bounty's headline `T_individual / T_batch` was the wrong single number to report; the correct framing is two numbers (throughput AND fee) with the recipient-distinctness dimension called out.

**Affects**: any payment-processor architecture that picks batching for "speed" — on this wallet, batching trades wall-clock for on-chain footprint.
**Captured in**: `results/S5_mode1_fresh_2026-05-30_with_fees.json` (self-send), `results/S5_distinct_with_fees.json` (distinct), `results/S5_mined_verification.json` + `results/S5_distinct_mined_verification.json` (chain confirmation of every claimed-ok tx).

---

## 8. Initial scenario-runner numbers without repetition or chain verification badly mislead

Three things I caught **on my own work** that anyone running the bounty should also catch:

1. **N=1 sample sizes overshoot**. Single-shot S4 at N=128 reported 12.9 tx/s. Three repeated runs at N=128 produced 7.64, 7.74, 7.95 tx/s (stdev 0.16). The single-shot was an outlier by ~60%. See `results/S4_variance_2026-05-31.json`.
2. **Cache warmth dominates rescan throughput**. Three S2/S3-shape recovery runs of the same shape produced 24,344 / 37,025 / 44,914 blocks/sec — an 85% spread from OS page cache state alone. Per-UTXO detection cost was negligible relative to walk time.
3. **"Throughput" claims need chain verification**. Our S4 results said 248 CoinSplit calls succeeded. Post-facto `GetTransactionInfo` poll (`scripts/verify_s4_mined.py`, with a corrected enum mapping I had to debug — `TX_MINED_CONFIRMED` is value 6, not 7) confirmed 248/248 in chain. We had no way to know without that follow-up — `Transfer` and `CoinSplit` return success on mempool acceptance, not on chain inclusion.

**Affects**: the credibility of any single-shot benchmark in this domain.
**Fix in the spec**: require all measurements to be repeated ≥3 times and reported with median + spread, and require chain-verification of every claimed-ok tx via post-facto `GetTransactionInfo` poll before throughput numbers are published.

---

# What changed since the first-pass write-up

The initial findings list had 19 entries. The folds:

- #2 + #6 (both `FundsPending` variants) → now #6 (same root cause, three shapes)
- #5 (NotEnoughFunds when amount > UTXO) → folded into #6 (same wallet-UTXO-state surface)
- #9 (seed encodes birthday) → folded into #2 (it's the workaround for #2, not a separate pain)
- #12 (N=32 cliff) + #14 (cliff was UTXO depth) → folded into #8 (the lesson is N=1 is not a benchmark; the cliff finding was self-corrected from better data)
- #13 (post-stress 60 s recovery) → folded into #6 (FundsPending at a different scale)
- #15 (S5 1.07× headline) + #16 (S5 fee 10.65×) → folded into #7 (now with the distinct-recipient flip too)
- #18 (Mode 3 stall at unsigned_tx_creator) → folded into #1 + #3 (downstream of those two upstream issues)
- #19 (Mode 3 stall at signer, production confirmation) → folded into #1 (it IS the production confirmation of #1)
- #3 (GetState proto drift) + #4 (`has_done_initial_validation` not `is_bootstrapped`) → folded into onboarding-friction, not wallet pain — the proto file in `tari/applications/minotari_app_grpc/proto/wallet.proto` is correct; the friction is for third-party docs.
- #8 (Mode 2 zombie daemon after re-scan) → folded into #2 (it's #2's downstream symptom)
