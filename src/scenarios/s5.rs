// S5 - Payment processor: batch vs. individual comparison.
//
// Purpose: Compare the cost of sending M payments in one batched transaction
// versus K individual transactions.  This measures whether batch construction
// is actually more efficient than serialised individual sends.
//
// Procedure:
//   Batch path:
//     1. Build M recipients (all using wallet's own address for simplicity).
//     2. Call batch_send(recipients) - single 1-to-M transaction.
//     3. Record construction + submission time.
//
//   Individual path:
//     1. Loop K times, calling send_to(address, amount) serially.
//     2. Record total time for all K sends.
//
// Result: BatchResult with both timings and speedup_factor.
//
// Note: The "not engineering around wallet pain" principle from the spec means
// we should NOT use any tricks to speed up the individual path.  The point is
// to surface real throughput numbers.

use crate::config::BenchmarkConfig;
use crate::metrics::{BatchResult, ScenarioResult};
use crate::modes::WalletMode;
use anyhow::Result;
use std::time::Instant;
use tracing::info;

pub async fn run(
    mode: &mut dyn WalletMode,
    config: &BenchmarkConfig,
    result: &mut ScenarioResult,
) -> Result<()> {
    let p = &config.params;
    let self_address = mode.get_address().await?;
    let amount: u64 = 1000; // 1000 µT per recipient

    // ── Batch path ────────────────────────────────────────────────────────────
    info!("[S5] batch send: M={} recipients", p.s5_m);
    let batch_balance = mode.get_balance().await?;
    let needed_batch = amount * p.s5_m as u64 + 100_000;
    if batch_balance < needed_batch {
        anyhow::bail!(
            "[S5] insufficient balance for batch: {} µT < {} µT",
            batch_balance, needed_batch
        );
    }

    let recipients: Vec<(String, u64)> = (0..p.s5_m)
        .map(|_| (self_address.clone(), amount))
        .collect();

    let batch_start = Instant::now();
    let batch_tx_id = mode.batch_send(&recipients).await?;
    let batch_elapsed = batch_start.elapsed();

    info!(
        "[S5] batch send complete: tx={} in {:.2?}",
        batch_tx_id, batch_elapsed
    );
    result.add_timing("batch_send", batch_elapsed);

    // ── Individual path ───────────────────────────────────────────────────────
    info!("[S5] individual send: K={} transactions", p.s5_k);
    let indiv_balance = mode.get_balance().await?;
    let needed_indiv = amount * p.s5_k as u64 + 100_000;
    if indiv_balance < needed_indiv {
        anyhow::bail!(
            "[S5] insufficient balance for individual sends: {} µT < {} µT",
            indiv_balance, needed_indiv
        );
    }

    let indiv_start = Instant::now();
    let mut indiv_sent = 0u32;
    for i in 0..p.s5_k {
        match mode.send_to(&self_address, amount).await {
            Ok(tx_id) => {
                indiv_sent += 1;
                info!("[S5] individual {}/{}: tx={}", i + 1, p.s5_k, tx_id);
            },
            Err(e) => {
                tracing::warn!("[S5] individual {}/{} failed: {}", i + 1, p.s5_k, e);
            },
        }
    }
    let indiv_elapsed = indiv_start.elapsed();

    info!(
        "[S5] individual sends complete: {}/{} in {:.2?}",
        indiv_sent, p.s5_k, indiv_elapsed
    );
    result.add_timing("individual_sends", indiv_elapsed);

    let speedup = if indiv_elapsed.as_secs_f64() > 0.0 {
        indiv_elapsed.as_secs_f64() / batch_elapsed.as_secs_f64()
    } else {
        1.0
    };

    result.batch_result = Some(BatchResult {
        batch_size: p.s5_m,
        batch_wall_secs: batch_elapsed.as_secs_f64(),
        individual_count: indiv_sent,
        individual_wall_secs: indiv_elapsed.as_secs_f64(),
        speedup_factor: speedup,
    });

    result.txs_sent = Some(1 + indiv_sent);

    info!("[S5] speedup factor: {:.2}x", speedup);
    Ok(())
}
