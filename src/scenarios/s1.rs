// S1 - UTXO build-up: doubling rounds followed by fan-out.
//
// Purpose: Stress-test construction and scanning with many small UTXOs.
//
// Procedure:
//   Phase A - Doubling (serial): `doubling_rounds` rounds of 1-in-2-out
//     transactions sent to self.  Each round doubles the UTXO count:
//       start: 1 UTXO  ->  after 6 rounds: 64 UTXOs
//
//   Phase B - Fan-out (serial): Split each of the 64 UTXOs into
//     `fanout_outputs_per_tx` outputs via CoinSplit.
//       64 inputs × 8 outputs = 512 UTXOs  (== volume_target)
//
//   After each phase, scan and wait for all transactions to confirm.
//
// Measurements:
//   - time per doubling round
//   - time for fan-out phase
//   - time to confirm all fan-out outputs
//   - total time
//   - final UTXO count

use crate::config::BenchmarkConfig;
use crate::metrics::ScenarioResult;
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
    let _self_address = mode.get_address().await?;

    // ── Phase A: doubling rounds ──────────────────────────────────────────────
    info!(
        "[S1] Phase A: {} doubling rounds (target {} UTXOs)",
        p.doubling_rounds,
        1u32 << p.doubling_rounds
    );

    let phase_a_start = Instant::now();

    for round in 0..p.doubling_rounds {
        let balance = mode.get_balance().await?;
        if balance < 2000 {
            anyhow::bail!(
                "[S1] insufficient balance for doubling round {}: {} µT",
                round, balance
            );
        }

        // Split into 2 equal outputs to self.
        let half = (balance / 2).saturating_sub(10_000); // leave a fee buffer
        let tx_id = mode.coin_split(half, 2).await?;
        info!("[S1] doubling round {}: coin_split tx={}", round, tx_id);

        // Wait for the split to confirm before the next round.
        mode.wait_for_confirmation(&tx_id, p.c_min, p.confirm_timeout_secs).await?;
    }

    let phase_a_elapsed = phase_a_start.elapsed();
    result.add_timing("doubling_phase", phase_a_elapsed);
    info!("[S1] Phase A done in {:.2?}", phase_a_elapsed);

    // ── Phase B: fan-out ──────────────────────────────────────────────────────
    let num_split_txs = 1u32 << p.doubling_rounds; // 64 for 6 rounds
    info!(
        "[S1] Phase B: fan-out {} inputs → {} outputs each, target {}",
        num_split_txs, p.fanout_outputs_per_tx, p.volume_target
    );

    let phase_b_start = Instant::now();
    let balance = mode.get_balance().await?;
    let amount_per_output = (balance / p.volume_target as u64).saturating_sub(10_000);

    let mut fanout_tx_ids = Vec::new();
    for i in 0..num_split_txs {
        let tx_id = mode.coin_split(amount_per_output, p.fanout_outputs_per_tx).await?;
        info!("[S1] fan-out tx {}/{}: id={}", i + 1, num_split_txs, tx_id);
        fanout_tx_ids.push(tx_id);
    }

    // Wait for all fan-out transactions to confirm.
    let confirm_start = Instant::now();
    for tx_id in &fanout_tx_ids {
        mode.wait_for_confirmation(tx_id, p.c_min, p.confirm_timeout_secs).await?;
    }
    result.add_timing("fanout_confirm", confirm_start.elapsed());

    let phase_b_elapsed = phase_b_start.elapsed();
    result.add_timing("fanout_phase", phase_b_elapsed);

    // ── Final scan to confirm UTXO count ─────────────────────────────────────
    let scan_start = Instant::now();
    let chain_height = mode.get_scanned_height().await?;
    mode.wait_for_scan_height(chain_height + 1, p.confirm_timeout_secs).await?;
    result.add_timing("final_scan", scan_start.elapsed());

    result.utxo_count = Some(p.volume_target);
    result.txs_sent = Some(p.doubling_rounds + num_split_txs);

    info!(
        "[S1] complete: ~{} UTXOs, phase_a={:.2?}, phase_b={:.2?}",
        p.volume_target, phase_a_elapsed, phase_b_elapsed
    );
    Ok(())
}
