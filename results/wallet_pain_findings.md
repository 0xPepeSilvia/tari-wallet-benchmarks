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

---

These findings will be expanded as Modes 2 and 3 are exercised. All are surfaced rather than engineered around per the bounty principle.
