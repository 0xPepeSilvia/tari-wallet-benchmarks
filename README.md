# tari-wallet-benchmarks

Reproducible wallet performance test harness for the Tari project.

Implements scenarios **B0** and **S0-S7** across three wallet modes as specified
in the [Wallet Performance Benchmarks bounty](https://github.com/tari-project/rfcs/issues/171).

## Modes

| Mode | Stack | How it works |
|------|-------|--------------|
| 1 | `minotari_console_wallet` | Spawns the daemon, drives via `tari.rpc.Wallet` gRPC |
| 2 | `minotari` (minotari-cli) | Subprocess per transaction: `create-unsigned-transaction` -> sign -> POST JSON-RPC |
| 3 | Payment Processor | Same daemon as Mode 1, uses multi-recipient `Transfer` gRPC call |

## Scenarios

| Scenario | Description |
|----------|-------------|
| B0 | Baseline genesis scan on empty wallet |
| S0 | Fund wallet to `a_fund` µT, wait for confirmation |
| S1 | UTXO build-up: 6 doubling rounds + fan-out to 512 UTXOs |
| S2 | Full genesis rescan (checkpoint 1, after S1) |
| S3 | Birthday rescan (checkpoint 1, after S1) |
| S4 | Concurrent construction: sweep N={8,16,32,64,128} workers |
| S5 | Payment processor: batch M=100 vs K=10 individual sends |
| S6 | Full genesis rescan (checkpoint 2, after S4/S5) |
| S7 | Birthday rescan (checkpoint 2, after S4/S5) |

## Quick start

### 1. Edit `benchmark.toml`

Set paths to your local binaries:

```toml
[binaries]
console_wallet = "C:/tari/minotari_console_wallet.exe"
base_node      = "C:/tari/minotari_node.exe"
minotari_cli   = "C:/tari/minotari.exe"

[node]
http_url = "http://127.0.0.1:18142"
```

### 2. Fund the wallets (S0)

S0 expects the wallet to receive `a_fund` µT (default: 10,000 tXTM).
Either:
- Set `TARI_BENCH_FUNDER_ADDRESS` and send funds out-of-band, then run S0
  which will poll until the balance arrives, or
- Pre-seed the wallet data directory with an already-funded wallet DB.

### 3. Run

```
# All modes, all scenarios
cargo run --release -- --config benchmark.toml

# Single mode, specific scenarios
cargo run --release -- --config benchmark.toml --modes 2 --scenarios B0,S0,S1,S2,S3

# List available options
cargo run --release -- --list-scenarios
```

### 4. Results

Results are written to `./benchmark-work/results/run-<uuid>.json`.

An intermediate partial file is written after each scenario so a partial run
is recoverable.

## Configuration reference

All parameters are documented in `benchmark.toml`.  The spec defaults are:

| Parameter | Default | Spec reference |
|-----------|---------|----------------|
| `a_fund` | 10,000,000,000 µT (10k tXTM) | A_fund |
| `c_min` | 3 | C_min |
| `volume_target` | 512 | volume_target |
| `doubling_rounds` | 6 | doubling_rounds |
| `fanout_outputs_per_tx` | 8 | fanout_outputs_per_tx |
| `s4_concurrency_levels` | [8,16,32,64,128] | N values |
| `s4_budget_secs` | 900 | T_budget |
| `s5_m` | 100 | M |
| `s5_k` | 10 | K |

## Environment variables

| Variable | Purpose |
|----------|---------|
| `TARI_BENCH_FUNDER_ADDRESS` | One-sided address of a funded wallet for S0 |
| `TARI_BENCH_BIRTHDAY_HEIGHT` | Block height the wallet was created (for S3/S7). Falls back to `tip - 1000`. |
| `TARI_BENCH_LOG` | Log level: `trace`, `debug`, `info`, `warn`, `error` (default: `info`) |

## Design notes

- **No engineering around wallet pain.** The harness measures real wallet
  behaviour under load.  It does not pipeline, pre-build, or cache transactions
  to hide latency.

- **Intermediate writes.** After every scenario the partial result JSON is
  flushed to disk so a hard failure doesn't lose prior measurements.

- **Mode independence.** Each mode gets its own wallet data directory and runs
  the full scenario sequence independently.  Results are directly comparable.

- **Address encoding.** Tari addresses are encoded as
  `bs58(byte[0]) + bs58(byte[1]) + bs58(bytes[2..])`, not standard base58 of
  the whole array.  The harness decodes gRPC raw bytes using this scheme.
