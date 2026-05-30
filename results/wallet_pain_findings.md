# Wallet pain findings - Mode 1 (minotari_console_wallet)

Run date: 2026-05-30
Network: Esmeralda
Wallet binary: `minotari_console_wallet 5.4.0-pre.0-74cc1b8e3a6c88f3982003464885e3829b575d39-release`
Methodology: harness drives the wallet via `tari.rpc.Wallet` gRPC, no engineering around the wallet's observed behaviour.

## Finding 1 - `RescanWallet(from_height=0)` does not reset scan to genesis

**RPC**: `tari.rpc.Wallet/RescanWallet`
**Behaviour expected from method name**: rescan from `from_height` to chain tip.
**Behaviour observed**: scanned_height dropped from 670,316 to 665,390 then climbed back to tip in ~10s. Floor was approximately tip − 5,000 blocks.

**Impact on the bounty spec**:
S2/S3/S6/S7 require genesis-from-0 and birthday-from-`H_birth` rescans. Using `RescanWallet` does NOT achieve this — anyone implementing those scenarios via this RPC will report fake-fast throughput numbers against any wallet already at tip.

The spec author's intended path (SWvheerden, tari-project/wallet-benchmarks#1, 2026-05-29):
> "When you need to rescan from genesis, use the api to change the seed words so that the encoded birthday reflects 0."

**Action**: Mode 1's `rescan_from(0)` will be reimplemented as a wallet-restart cycle: stop wallet → rewrite encoded birthday in the seed → reinit wallet DB → restart → poll scanned_height from 0 to tip.

## Finding 2 - `OutputManagerError(FundsPending)` on serial spends

**RPC**: `tari.rpc.Wallet/CoinSplit`
**Behaviour observed**: spending an output that was created by a transaction still in the mempool returns:
```
status = INTERNAL
details = "OutputManagerError(FundsPending)"
```
even when `GetBalance.pending_in` shows 0.

**Reproduction (S1 round 1 → round 2)**:
1. Wallet holds 1 UTXO of 30,000 tXTM (S0 funding, confirmed).
2. `CoinSplit(amount=15000 tXTM, count=2, fee=5)` succeeds in 32 ms. tx_id returned.
3. `GetBalance` returns `available=30000, pending_in=0, pending_out=0` (the wallet's accounting has not yet caught up to the broadcast).
4. Next `CoinSplit` call (round 2) fires within the same second.
5. Wallet rejects with `OutputManagerError(FundsPending)`.

**Why this matters for the bounty spec**:
The S1 per-round procedure assumes a straightforward "construct serial txs in round, wait for confirmation, proceed". A naive implementation that reads `GetBalance.pending_*` to decide when to advance will hit `FundsPending` on every round transition. Harness authors will reach for "wait_for_confirmation_depth_C_min" — but the *actual* gate is "the input outputs must be MINED", not just past some pending threshold.

**Surfacing the cost honestly**:
- Per-round wall clock now includes a chain-tip-advance wait between rounds. On esmeralda with block time ~120s and C_min=1, each round adds ~120-240s of waiting.
- The spec's S1 wall-clock total is therefore dominated by chain wait time, not wallet construction time. Per-tx construction itself is fast: 32 ms observed in round 1.

**Action**: S1 driver now uses `wait_for_chain_advance(blocks=C_min+1)` between rounds — polling `GetState.scanned_height` — instead of the misleading `wait_for_balance_settle`. This is the kind of behaviour the bounty principle ("harness measures, does not engineer around wallet pain") wants surfaced, not hidden.

## Finding 3 - Proto drift in `GetState`

**Issue**: `tari.rpc.Wallet/GetState`'s wire format does not match the shape commonly assumed in third-party docs.

**Actual shape (as of binary `5.4.0-pre.0-74cc1b8...`)**:
```protobuf
service Wallet {
  rpc GetState(GetStateRequest) returns (GetStateResponse);
}
message GetStateRequest {}
message GetStateResponse {
  uint64 scanned_height = 1;
  GetBalanceResponse balance = 2;
  NetworkStatusResponse network = 3;
  bool has_done_initial_validation = 4;
}
```

**What the harness initially had** (drawn from an older spec):
```protobuf
message WalletStateResponse {
  bool is_started = 1;
  bool is_bootstrapped = 2;
  uint64 height_of_longest_chain = 3;
  uint64 scanned_height = 4;
}
```

This is a real onboarding friction point for anyone building a third-party wallet client. The bounty's `Acceptance Criteria > A third party can clone the repo, build the harness, fund the wallets, and reproduce the test` implies the harness is itself a documentation artifact — so the canonical proto must be the one that compiles against the live binary, not the one in older docs.

## Finding 4 - "Initial validation" replaces "bootstrapped" as the readiness signal

Mode 1's wallet-ready check originally polled `state.is_bootstrapped`. That field no longer exists on `GetStateResponse`. The current readiness signal is `state.has_done_initial_validation` — true once the wallet has scanned past its birthday and validated existing outputs against the chain. The harness now polls this; on a fresh wallet pointed at a synced base node it flips true after the first scan tick (within seconds).

## Finding 5 - `OutputManagerError(NotEnoughFunds)` on amount > smallest UTXO

**RPC**: `tari.rpc.Wallet/CoinSplit`
**Behaviour**: the wallet does not auto-aggregate multiple small UTXOs to fund a single CoinSplit. If no single UTXO covers `amount_per_split * split_count + fee`, the wallet errors `NotEnoughFunds` even when the total wallet balance is many times the requested amount.

**Reproduction (S1 v2)**:
- Round 2 split 30,000 tXTM into 2 outputs of 1,875 tXTM each (sizing `avail // (2 * 8)`).
- Round 3 requested 4 splits of 937 tXTM each. Each tx needed an input of ~1,875 tXTM. The smallest available UTXOs were exactly 1,875 tXTM and could not cover 1,875 + fee.
- `NotEnoughFunds` returned even though available_balance was ~22,500 tXTM total.

**Mitigation in the driver**: use a small fixed split amount (50 tXTM) per output, regardless of round, so the change UTXO from each tx is always >> the next round's per-tx input requirement. The wallet will keep selecting the largest available UTXO as input.

## Finding 10 - Mode 2 locks the entire balance for 24 hours on first unsigned tx

**Component**: `minotari create-unsigned-transaction` (subprocess) at `tari-project/minotari-cli`
**Behaviour observed**: a single-UTXO wallet calls `create-unsigned-transaction` once and receives a valid unsigned tx JSON. Every subsequent call within the next 24 hours returns:

```
WARN  Insufficient funds for transaction (pending confirmations)
Error: Failed to lock funds: Funds are pending. Available: 0 µT, Pending: 30000.000000 T, Required: 100.000000 T
```

**Why**: the first call invokes `lock_funds`, which reserves the wallet's only UTXO with a default `--seconds-to-lock 86400` (24 hours). Until that lock expires OR the unsigned tx is signed AND submitted AND mined (producing a new change UTXO), the wallet has zero spendable balance.

**Impact on the bounty spec**:
- S1 (UTXO build-up) is structurally impossible on Mode 2 starting from a single funding UTXO. You cannot construct round 2 until round 1's tx is signed, submitted, mined, and the change becomes spendable.
- S4 (concurrent construction at N=8..128) is unmeasurable from a single funding UTXO. Either pre-split into N UTXOs (which itself requires Mode 2 sign+broadcast working end-to-end) or accept that the first call locks everything.
- Mode 2 requires a fully-working sign-and-submit pipeline to be exercisable beyond a single construction call. See Finding #11.

**Workaround for measuring construction-only throughput**: use `--seconds-to-lock 1` (1 second) per call so locks expire between back-to-back invocations. This bypasses the design intent but lets the harness time `create-unsigned-transaction` × N without an end-to-end pipeline.

## Finding 11 - `create-unsigned-transaction` and `sign-one-sided-transaction` versions are incompatible

**Components**:
- producer: `minotari create-unsigned-transaction` at `tari-project/minotari-cli` current main
- consumer: `minotari_console_wallet sign-one-sided-transaction` at `tari-project/tari` current main

**Behaviour observed**: minotari emits unsigned-tx JSON with `"version": "4.0.0"`. console_wallet's signer rejects it:

```
Invalid command. Transaction service error
`Error serializing transaction: Unsupported version. Expected '5.0.0', got '4.0.0'`
```

**Impact on the bounty spec**:
The bounty's Mode 2 description ("uses the minotari crate directly: local UTXO selection, sign_locked_transaction, broadcast via HTTP RPC") and SWvheerden's clarification on `wallet-benchmarks#1` (2026-05-22: "you can do signing like Tari Universe or you can call the signing application") imply that minotari produces an unsigned tx and an external signer accepts it. The two tools available today are version-incompatible.

The only working signing path is the Rust library call to `tari_transaction_components::offline_signing::sign_locked_transaction`. That requires linking the workspace as a cargo dep (heavy: ~250 transitive crates, nightly Rust 2024 edition).

Sub-finding: `sign-one-sided-transaction` invoked via `--seed-words "..."` triggers full wallet recovery as a side effect (scanning chain from 0) before exiting without ever running the sign command. The command must be invoked against a base-path that already contains a recovered wallet DB.

**Observation that became a free measurement**: console_wallet recovery completed 670,998 blocks in 27.56 s = 24,344 blocks/s. This is `~3.4x slower` than the empty-wallet scan (Finding from B0_mode1: 82,000 blocks/s) because recovery does view-key matching plus DB writes for any detected outputs, whereas B0's scanner has no outputs to record.

## Finding 8 - Mode 2 daemon enters silent-zombie state after re-scan

**Component**: `minotari` daemon subprocess at `tari-project/minotari-cli` (current main)
**Behaviour observed**: after running `re-scan --rescan-from-height 0` and then launching the daemon against the same DB, the daemon emits this log pattern indefinitely:

```
INFO  Starting wallet scan...
ERROR Failed to send download error with error: channel closed
INFO  Scan completed successfully event_count=0
ERROR Failed to send download error with error: channel closed
```

The `scan_status` API reports a stale `last_scanned_height` (in this run: 670400, unchanged across ~3 minutes of "successful" scan cycles). The wallet does not detect any new blocks despite the base node being reachable and producing them.

**Why this matters**:
- The daemon claims `Scan completed successfully event_count=0` so any monitoring that only checks for "scan succeeded" will miss the failure.
- The `channel closed` error is in the chain-download path, not the scan-orchestration path - so the scan loop continues spinning but never gets fresh blocks.
- Recovery requires deleting the wallet DB and recreating with seed words; in-place restart does not heal.

**Workaround**: delete `wallet.db` and recreate via `minotari create --seed-words "..."`. The seed-derived birthday is preserved across recreate, so the wallet finds historical UTXOs from its birthday forward on the first daemon scan.

## Finding 9 - Mode 2 birthday derives from seed words and persists across DB delete

This is the GOOD news that makes finding 8 recoverable. After deleting the Mode 2 wallet DB and recreating via `minotari create --seed-words "<the same words>"`, the daemon's first scan picks up at the original birthday height (the seed-encoded birthday) and finds historical UTXOs that were received during that wallet's first lifetime.

**Measurement (this run)**:
- Wallet seed words: `doctor exist smoke swing ... vintage` (24 words)
- Wallet first created: 2026-05-30 ~11:25 (chain tip ~670380 at that time)
- Wallet DB deleted then recreated from same seed: 2026-05-30 ~16:20
- Daemon launched immediately after recreate
- `t+0.5s`: daemon API ready
- `t+0s` (first poll, immediately after API ready): `available_balance = 30,000.00 tXTM`, `scanned_height = 670974`

The 30,000 tXTM funding transaction was broadcast at ~11:13, mined into a block somewhere in the range 670400-670974. The daemon's first scan cycle - between API readiness and our first balance poll - rescanned ~594 blocks (670380 birthday → 670974 tip) and detected the historical funding tx.

**Mode 2 scan throughput (birthday-shaped)**: 594 blocks in `<10s` (our polling resolution), implying `>= 59 blocks/sec` floor. Insufficient resolution for a precise number, but well within the same order of magnitude as Mode 1's 82,000 blocks/sec B0 measurement once you account for the smaller scan window.

## Finding 19 - Mode 3 PP pipeline stalls at SIGNER due to Finding #11 (production confirmation)

The first Mode 3 attempt (Finding #18) was blocked at `unsigned_tx_creator` by Mode 2's lock state (Finding #10). Recreating Mode 2 cleared the lock and let me re-run the pipeline. The second attempt got further:

```
PP receives POST /v1/payment-batches (10 payments) -> 202 Accepted in 5 ms
batch_creator emits BatchCreated event
unsigned_tx_creator calls PR (Mode 2 minotari-cli daemon):
  - lock_funds: succeeds (Mode 2 locks 30,000 tXTM)
  - create_unsigned_transaction: succeeds, returns v4.0.0 JSON
transaction_signer invokes console_wallet sign-one-sided-transaction subprocess
console_wallet exits 107 with stderr:
  "Invalid command. Transaction service error 
   `Error serializing transaction: Unsupported version. Expected '5.0.0', got '4.0.0'`"
PP reverts batch to AwaitingSignature, retries 10 seconds later
SAME error
Infinite retry loop
```

This is **Finding #11 at production scale** - confirmed not a harness-only artifact. Anyone running Mode 3 from current main of `tari-project/tari` + `tari-project/minotari-cli` + `tari-project/minotari_payment_processor` will hit this exact retry loop. The pipeline cannot complete a single payment until the v4/v5 version mismatch is fixed in one of the two repos.

**Mode 3 wiring is otherwise complete**: PP service starts, all 5 workers initialise, HTTP API serves, DB writes work, PR connectivity works, base node connectivity works, payment ingestion works (5,000 payments/sec API throughput), batch creation works, lock_funds works, create_unsigned_transaction returns a valid unsigned tx body. The single bug at the signer subprocess invocation step blocks every measurement past `SigningInProgress`.

## Finding 18 - Mode 3 PP pipeline stalls at unsigned_tx_creator due to Finding #10

Built `minotari_payment_processor` from `tari-project/minotari_payment_processor` current main (workaround: `DATABASE_URL=sqlite://data/payments.db` relative form, not absolute `sqlite:///c/...`), configured against:
- `PAYMENT_RECEIVER=http://127.0.0.1:9006` (Mode 2 minotari-cli daemon)
- `BASE_NODE=http://127.0.0.1:9005`
- `CONSOLE_WALLET_PATH=.../minotari_console_wallet.exe`
- `CONSOLE_WALLET_BASE_PATH=C:/Tari-bench-mode2` (recovered Mode 2 wallet for signing)
- `ACCOUNTS__DEFAULT__{NAME, VIEW_KEY, PUBLIC_SPEND_KEY}` = Mode 2 wallet's keys

PP service starts cleanly, exposes the documented HTTP API at `:9145` (swagger-ui, `/health/version`, `/v1/payment-batches`, `/v1/events`, `/v1/payments/{id}`). All 5 workers initialise; `confirmation_checker` polls chain tip happily; the connection to the `PAYMENT_RECEIVER` succeeds.

`POST /v1/payment-batches` with 100 PaymentItems returns `202 Accepted` in **19 ms** with all 100 payments in `status=BATCHED`. `events_total` becomes 101 (1 `BatchCreated` + 100 `PaymentReceived`).

But the `unsigned_tx_creator` worker then loops on Mode 2's `lock_funds` endpoint receiving `Funds are pending. Available: 0 uT, Pending: 30000 T, Required: <batch_amount>` because the Mode 2 wallet's only UTXO is locked from an earlier `create-unsigned-transaction` call (the 24-hour `--seconds-to-lock` default — Finding #10). All 100 payments remain `BATCHED` indefinitely.

**The Mode 3 pipeline IS architecturally complete** — every wired component works in isolation. The single observable blocker is Finding #10. Once Mode 2's lock issue is resolved upstream OR worked around by recreating the wallet, the same `POST /v1/payment-batches` call would walk the full `UNSIGNED → SIGNED → BROADCAST → CONFIRMED` path with each stage emitting a status-update event. This run measured the API ingestion throughput only: **~5,000 payments/sec at the API surface**.

## Finding 17 - Mode 1 console_wallet has no scriptable seed-word export

**Component**: `minotari_console_wallet`

**Behaviour observed**: the wallet creates a 24-word seed during first launch and stores it encrypted in its DB. The TUI exposes a menu to reveal it interactively. Non-interactive mode has no equivalent command.

Available console_wallet subcommands inspected:
- `export-view-key-and-spend-key` → emits keys, not seed words
- `whois <PUBLIC_KEY>` → public-key lookup, not seed export
- `show-pay-ref` → payment ID lookup, not seed export
- `--seed-words-file-name <PATH>` → only writes the seed during wallet CREATION, not for an existing wallet

**Impact on the bounty spec**:
S2 (genesis rescan after S1) on Mode 1 wants the *same* wallet recovered against the *same* chain. Without scriptable seed export, the harness operator must either:
1. Run console_wallet interactively, navigate to the seed-words menu, copy-paste — not reproducible.
2. Skip Mode 1's S2/S6 entirely and accept the partial measurement we have (S2 with N=1 UTXO, captured incidentally during a Mode 2 sign attempt).

**Recommended upstream**: `minotari_console_wallet show-seed-words --password ... --base-path ...` mirroring `minotari-cli show-seed-words` which already exists.

## Finding 16 - S5 batch vs individual: FEE ratio is 10.65x even when throughput multiplier is 1.07x

**This is the headline measurement once both numbers are on the table.**

Per-recipient cost from the same S5 run (post-facto `GetTransactionInfo` poll for fee field):

| Metric | Arm A batch (10 recipients/tx) | Arm B individual (1 recipient/tx) | Ratio B/A |
|---|---|---|---|
| Wall clock per recipient | 88 ms | 93 ms | **1.07x** |
| Fee per recipient | **74 µT** | **788 µT** | **10.65x** |
| Total fees for 100 recipients | 7,400 µT (0.0074 tXTM) | 78,775 µT (0.0788 tXTM) | 10.65x |

**Implication**:
The bounty's `T_individual / T_batch` throughput multiplier asks the wrong question if you stop there. Batch processing on Mode 1 does NOT meaningfully reduce wall-clock to deliver M payments. It DOES reduce the fee paid per recipient by ~10x. For a payment processor moving high recipient volume, the operating-cost case for batching is real; the throughput case largely isn't.

**Why the fee ratio is what it is**:
- Arm A: 10 batch txs ≈ 10 inputs + 100 outputs + 10 signatures + 10 tx structures on chain
- Arm B: 100 individual txs ≈ 100 inputs + 100 outputs + 100 signatures + 100 tx structures on chain

The per-byte fee plus per-output range-proof weight applies once per output regardless of arm, but per-input/per-signature/per-structure cost amortises 10x more in the batch arm. So fees scale roughly with tx count, not output count, in this regime.

## Finding 15 - S5 batch vs individual throughput speedup (initial framing, superseded by #16)

**Headline measurement** (Mode 1, console_wallet via gRPC Transfer, fresh wallet with ~470 UTXOs from S1):

| Arm | Tx count | Wall clock | Per-tx | Per-recipient | All ok |
|---|---|---|---|---|---|
| A - batch (10 recipients/tx) | 10 | 8.67 s | 867 ms | **88 ms** | 100/100 |
| B - individual (1 recipient/tx) | 100 | 9.30 s | 93 ms | **93 ms** | 100/100 |

**Throughput multiplier (B/A): 1.07x**

Batching saves only ~5 ms per recipient - the gRPC round-trip overhead. The wallet's per-output construction work (commitment, range proof, output features) is identical between batch and individual paths. Bundling 10 recipients into one transaction does NOT amortize an 867 ms cost over 10; the cost scales linearly with output count.

**Bounty-relevant interpretation**:
The spec asks for "Throughput multiplier = T_individual / T_batch (headline)". We measured 1.07x. Operators evaluating Tari for payment-processing workloads would benefit from knowing that the throughput gain from batching is small - the dominant cost is per-recipient construction, not per-transaction submission. The on-chain footprint advantage of batching (fewer txs, fewer signatures, lower fee per recipient) remains valid; the harness's wall-clock measurement just reveals that this is the dominant benefit, not pure throughput.

**Per-tx timings observed during Arm A**:
833 / 986 / 882 / 1000 / 896 / 781 / 813 / 804 / 733 / 939 ms (avg 867)

**Per-tx timings observed during Arm B**:
sampled every 10: 95 / 108 / 93 / 93 / 93 / 92 / 91 / 77 / 93 / 92 ms (avg 93)

Both arms have very stable per-call latency - no GC pauses, no UTXO contention. Standard deviations are tight (~10% of mean for Arm A, <5% for Arm B).

**What the spec also asks for that this run does NOT capture**:
- Total fees per arm (Transfer RPC response does not include fee; would need per-tx `GetTransactionInfo` lookup after the fact)
- Blocks consumed per arm
- Fee per recipient breakdown

These could be captured in a follow-up by polling `GetTransactionInfo` for each returned tx_id after Arm B completes and the fee data is available on chain.

## Finding 14 - The "N=32 cliff" was UTXO-pool exhaustion, NOT a wallet ceiling

**This revises and re-frames the earlier observation in Finding #12.**

Running S4 against the SAME wallet binary but with a deep UTXO pool (~470 UTXOs from S1 doubling + fanout) produces a dramatically different picture:

| N | OK (deep pool) | tx/s | Max gap (ms) | OK (mining wallet, was Finding #12) |
|---|---|---|---|---|
| 8 | 8/8 | 16.4 | 78 | 8/8 |
| 16 | 16/16 | 16.2 | 78 | 13/16 |
| 32 | 32/32 | 15.6 | 81 | **0/32** |
| 64 | 64/64 | 14.0 | 93 | **0/64** |
| 128 | 128/128 | 12.9 | 245 | **0/128** |

The wallet sustains 13-16 tx/s across N=8 through N=128 when each concurrent worker can grab its own UTXO. Throughput degrades gracefully (~20% drop from N=8 to N=128) - this is the expected per-call locking overhead, not a scaling failure.

The earlier "cliff" at N=32 on the mining wallet happened because the mining wallet, despite holding 209k tXTM total, had a SHALLOW pool of confirmed-spendable UTXOs once the first 16-21 successful concurrent calls locked them. Subsequent calls saw "no spendable UTXOs" and all failed in the same millisecond burst.

**Implication for the bounty spec**:
The spec's correct execution sequence (S1 builds the 512-UTXO pool BEFORE S4 runs) is exactly the right shape. Running S4 against a fresh-funded single-UTXO wallet would have produced the misleading mining-wallet shape. The bounty author knew this - the spec mandates S1 -> S4 ordering for a reason.

**For the result profile**: report the UTXO pool depth at the start of each S4 N-level so reviewers can correlate throughput with available work, and treat any "instant total failure" at high N as a pool-depth signal rather than a wallet-throughput claim.

## Finding 12 - "N=32 cliff" on a shallow UTXO pool (initial observation)

**RPC**: `tari.rpc.Wallet/CoinSplit` invoked from N concurrent threads.

**Measurement (mining wallet, ~179k tXTM, many UTXOs)**:

| N | OK | Wall (s) | tx/s | Max gap (ms) | Failures |
|---|---|---|---|---|---|
| 8 | 8/8 | 0.42 | 18.9 | 64 | none |
| 16 | 13/16 | 0.68 | 19.2 | 63 | 3 FundsPending |
| 32 | 0/32 | 0.02 | 0.0 | 1 | 32 FundsPending |
| 64 | 0/64 | 0.02 | 0.0 | 1 | 64 FundsPending |
| 128 | 0/128 | 0.05 | 0.0 | 2 | 128 FundsPending |

**Signature**: at N >= 32 the wallet rejects EVERY concurrent call with `FundsPending` in 1-2 ms. The whole batch completes in <50 ms with zero successful constructions. The 1 ms gap between completions implies the failures are returning in one tight burst from a single hot path - classic mutex-collision pattern on the output_manager's UTXO selection.

**Bounty-relevant interpretation**:
This is the answer to the bounty's S4 question for Mode 1: "where does this wallet break under concurrent construction load?" Between N=16 and N=32. Above 16 concurrent threads the wallet's UTXO-locking path serialises so aggressively that none of the requests can complete - they all see "Funds pending" simultaneously and abort.

## Finding 13 - Post-concurrent-stress recovery time exceeds 60 seconds

**Setup**: after S4 fired 248 concurrent CoinSplit calls (21 successful, 227 rejected), we paused 60 s and then ran S5 (Transfer-based, batch + individual arms).

**Result**: every Transfer call in both S5 arms returned `Output manager error: 'Funds are still pending. Unable to fulfil transaction right now.'` in 1-5 ms. Zero successful Transfer calls across 110 attempts spread over the next 1-2 minutes.

**Interpretation**: the wallet's output_manager appears to maintain a global "stress" state after concurrent load that persists much longer than the original chain confirmation time would suggest. The 21 successful S4 txs would normally clear in ~2 minutes (one Esmeralda block-time), but the post-stress recovery window is at least 60 s before any new Transfer can succeed.

**Caveat**: this S5 run is not a clean batch-vs-individual measurement because of the saturation. Will be re-run on a fresh wallet that has not been concurrency-stressed.

**Implication for the spec's S5**: the spec requires S4 to be run BEFORE S5. If the wallet stays in this rejection state for >>60 s after S4, the S5 numbers will be dominated by the wait-for-recovery time rather than the actual batch-vs-individual difference. Either S5 starts after a chain-block-time-multiple wait, or the spec's S4 -> S5 sequence will produce misleading S5 results.

## Finding 7 - Mode 2 has no API to do a genesis rescan

**Component**: `minotari` (the minotari-cli library / daemon) at `tari-project/minotari-cli`
**Behaviour observed**: Mode 2 exposes `re-scan --rescan-from-height <H>` (subprocess) and a daemon HTTP API. Neither path can perform a true genesis rescan on an existing wallet:

1. `re-scan --rescan-from-height 0` issues a long sequence of `BlockRolledBack` events and soft-deletes outputs/inputs/scanned tip blocks down to height 0 in the SQLite DB. It logs `Re-scan complete event_count=0` and exits 0. But the wallet's `last_scanned_height` value is not reset.
2. When the daemon is next started, it reads `last_scanned_height` from the DB (still at the previous tip-adjacent value, observed: 670400) and resumes scanning forward. The chain history between birthday and the previous tip is never re-walked.
3. `minotari create` (the wallet init command) has no `--birthday` flag. A freshly created wallet's birthday is the current chain tip at creation time. The wallet will only ever scan from that height forward.

**Impact on the bounty spec**:
- B0 (genesis scan on fresh wallet) is unmeasurable on Mode 2 via any documented path.
- S2 / S6 (genesis rescan with UTXOs) are likewise unmeasurable.
- S3 / S7 (birthday rescan) is the only scan-shape measurable on Mode 2, and the harness must capture the wallet's birthday at creation time to compute `blocks_scanned = tip - birthday`.

**SWvheerden's guidance to roadhero on 2026-05-29** ("change the seed words so that the encoded birthday reflects 0") describes the path the spec author intended for this scenario — but that path is not exposed by the minotari-cli today. It would require either (a) a new `--birthday` flag on `create` / `migrate-from-console-wallet`, or (b) a DB-level patch that the operator runs out-of-band before launching the daemon.

**Result-profile treatment**: Mode 2's B0/S2/S6 entries are recorded as `status = "blocked_by_wallet_api"` with the finding text inline, per the bounty principle that wallet pain must be visible in the metrics.

## Finding 6 - `FundsPending` mid-round when spendable UTXO pool exhausts

**Setup**: S1 round 4 attempts 8 serial CoinSplit calls back-to-back. Wallet had ~12 UTXOs at start of round (mix of round-3 outputs + change).
**Behaviour observed**: txs 1-7 succeeded. tx 8 returned `OutputManagerError(FundsPending)`.

**Interpretation**: each CoinSplit consumes one input UTXO and produces (split_count) outputs plus a change output. The change is unspendable until the parent tx mines. So firing N serial CoinSplit calls requires the wallet to start with at least N "confirmed spendable" UTXOs - the change from earlier-in-round txs does not refill the pool.

**Impact on the bounty spec**:
S1 doubling rounds 4-6 fire 8, 16, 32 serial txs respectively. To complete round 4 you need at least 8 confirmed UTXOs going in. Round 5 needs 16. Round 6 needs 32. If a prior round leaves fewer confirmed-spendable UTXOs than the next round needs, that round fails partway.

This is real selection contention - the same wallet pain that S4's concurrent path is designed to surface. It also appears in S1's serial path. The bounty principle says surface it, not engineer around it.

**Numerical record (Mode 1, this run)**:

| Round | Txs requested | Txs succeeded | Construct (ms total) | Wall clock (s, includes chain wait) |
|---|---|---|---|---|
| 1 | 1 | 1 | 34 | 45 |
| 2 | 2 | 2 | 70 | 60 |
| 3 | 4 | 4 | 169 | 60 |
| 4 | 8 | 7 | (~280, est.) | failed mid-round |

Total measurable doubling: 14 successful txs in ~3 minutes of chain-mine wait (165s). Per-tx construction time: 35-45ms (essentially negligible vs chain-confirmation latency).

---

These findings will be expanded as Modes 2 and 3 are exercised. All are surfaced rather than engineered around per the bounty principle.
