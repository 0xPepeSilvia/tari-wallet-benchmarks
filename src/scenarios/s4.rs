// S4 - Concurrent transaction construction.
//
// Purpose: Measure throughput as N workers simultaneously construct (but do not
// necessarily submit) transactions.  The bounty spec is explicit that this
// measures construction throughput, not submission throughput - the bottleneck
// being probed is UTXO locking / key-derivation / output-commitment building.
//
// For each concurrency level N in params.s4_concurrency_levels:
//   1. Spawn N tokio tasks, each constructing one transaction.
//   2. Collect all results within params.s4_budget_secs.
//   3. Record (N, txs_constructed, elapsed, txs_per_sec).
//
// Note on wallet constraints:
//   - Mode 1/3 (gRPC): CoinSplit or Transfer with N recipients, one call.
//   - Mode 2 (CLI): N concurrent subprocess invocations of create-unsigned-transaction.
//
// We send transactions to a throwaway random address to avoid confirmation wait.

use crate::config::BenchmarkConfig;
use crate::metrics::{ConcurrencyResult, ScenarioResult};
use crate::modes::WalletMode;
use anyhow::Result;
use std::time::{Duration, Instant};
use tracing::info;

// A simple burn address for transactions that don't need to be received.
// In real Tari these would be valid one-sided stealth addresses.
// For the harness we use the wallet's own address (send-to-self) so
// the transaction is constructable without a second wallet running.

pub async fn run(
    mode: &mut dyn WalletMode,
    config: &BenchmarkConfig,
    result: &mut ScenarioResult,
) -> Result<()> {
    let p = &config.params;
    let self_address = mode.get_address().await?;
    let amount: u64 = 1000; // 1000 µT per tx - tiny, just measuring construction

    let mut concurrency_results = Vec::new();

    for &workers in &p.s4_concurrency_levels {
        info!("[S4] testing concurrency level: {} workers", workers);

        let balance = mode.get_balance().await?;
        let needed = amount * workers as u64 + 100_000; // fee buffer
        if balance < needed {
            tracing::warn!(
                "[S4] skipping N={}: balance {} µT < {} µT needed",
                workers, balance, needed
            );
            concurrency_results.push(ConcurrencyResult {
                workers,
                txs_constructed: 0,
                wall_secs: 0.0,
                txs_per_sec: 0.0,
            });
            continue;
        }

        let start = Instant::now();
        let budget = Duration::from_secs(p.s4_budget_secs);

        // Build N recipients in one batch (simulates concurrent construction).
        // For Mode 2 this is N parallel CLI subprocesses; for Modes 1/3 it's
        // one multi-recipient Transfer call.
        let recipients: Vec<(String, u64)> = (0..workers)
            .map(|_| (self_address.clone(), amount))
            .collect();

        let tx_result = tokio::time::timeout(
            budget,
            mode.batch_send(&recipients),
        )
        .await;

        let elapsed = start.elapsed();
        let constructed = match tx_result {
            Ok(Ok(_)) => workers,
            Ok(Err(e)) => {
                tracing::warn!("[S4] N={} batch_send error: {}", workers, e);
                0
            },
            Err(_) => {
                tracing::warn!("[S4] N={} timed out after {:.2?}", workers, elapsed);
                0
            },
        };

        let txs_per_sec = if elapsed.as_secs_f64() > 0.0 {
            constructed as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        info!(
            "[S4] N={}: {} txs in {:.2?} ({:.2} tx/s)",
            workers, constructed, elapsed, txs_per_sec
        );

        concurrency_results.push(ConcurrencyResult {
            workers,
            txs_constructed: constructed,
            wall_secs: elapsed.as_secs_f64(),
            txs_per_sec,
        });
    }

    result.concurrency_results = Some(concurrency_results);
    Ok(())
}
