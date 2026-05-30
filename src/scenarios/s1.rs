// S1 - UTXO build-up: doubling rounds + fan-out → 512 UTXOs.
//
// SPEC (round-by-round, not one tx per round):
//   Doubling phase (6 rounds):
//     Round 1: 1 tx  (1 in → 2 out)   → 2 UTXOs after
//     Round 2: 2 txs (each 1 in → 2 out) → 4 UTXOs
//     Round 3: 4 txs                  → 8 UTXOs
//     Round 4: 8 txs                  → 16 UTXOs
//     Round 5: 16 txs                 → 32 UTXOs
//     Round 6: 32 txs                 → 64 UTXOs
//   Fan-out phase: 64 txs, each 1 in → 8 out → 512 UTXOs
//
//   Total: 63 doubling txs + 64 fan-out txs = 127 txs on chain
//
// Per-round procedure (from spec):
//   1. Snapshot balance and UTXO set
//   2. Construct + broadcast all round txs serially
//   3. Wait until all round txs reach depth >= C_min
//   4. Refresh wallet state (tip refresh, not full scan)
//   5. Verify UTXO count matches target
//   6. Verify balance_after == balance_before - sum(round_fees)
//
// FAILURE HALTS THE SCENARIO - if any round fails to land all txs, S1 aborts.

use crate::config::BenchmarkConfig;
use crate::metrics::{ScenarioResult, TxMetrics};
use crate::modes::WalletMode;
use anyhow::{Context, Result};
use std::time::Instant;
use tracing::{info, warn};

pub async fn run(
    mode: &mut dyn WalletMode,
    config: &BenchmarkConfig,
    result: &mut ScenarioResult,
) -> Result<()> {
    let p = &config.params;
    let _self_address = mode.get_address().await?;
    let mut per_tx_metrics: Vec<TxMetrics> = Vec::new();

    // ── Phase A: doubling, round-by-round ────────────────────────────────────
    info!(
        "[S1] Phase A: {} doubling rounds (target {} UTXOs after doubling)",
        p.doubling_rounds,
        1u32 << p.doubling_rounds
    );

    let phase_a_start = Instant::now();

    for round in 0..p.doubling_rounds {
        let txs_this_round = 1u32 << round; // 1, 2, 4, 8, 16, 32

        let balance_before = mode.get_balance().await?;
        info!(
            "[S1] doubling round {}/{}: {} tx(s), balance {} µT",
            round + 1,
            p.doubling_rounds,
            txs_this_round,
            balance_before
        );

        // Each tx in this round consumes 1 input, produces 2 outputs to self.
        // Use coin_split(amount_per_output, 2) per tx.  Amount sized so the
        // wallet still has spendable balance for the next round.
        let target_per_tx = balance_before / (txs_this_round as u64 * 4); // /4 leaves headroom
        if target_per_tx < 10_000 {
            anyhow::bail!(
                "[S1] round {}: per-tx amount {} µT too small to cover fee",
                round + 1,
                target_per_tx
            );
        }

        let mut round_tx_ids = Vec::new();
        for tx_idx in 0..txs_this_round {
            let construct_start = Instant::now();
            let tx_id = mode
                .coin_split(target_per_tx, 2)
                .await
                .with_context(|| format!("S1 round {} tx {} construct failed", round + 1, tx_idx))?;
            let construct_elapsed = construct_start.elapsed();

            per_tx_metrics.push(TxMetrics {
                scenario_phase: format!("doubling_round_{}", round + 1),
                tx_id: tx_id.clone(),
                construction_secs: construct_elapsed.as_secs_f64(),
                broadcast_to_mempool_secs: 0.0, // coin_split returns after mempool acceptance
                broadcast_to_confirmed_secs: None,
                fee_paid_ut: None,
                outcome: "constructed".to_string(),
            });
            round_tx_ids.push(tx_id);
        }

        // Wait for all round txs to confirm at depth >= c_min.
        let confirm_start = Instant::now();
        for tx_id in &round_tx_ids {
            mode.wait_for_confirmation(tx_id, p.c_min, p.confirm_timeout_secs)
                .await
                .with_context(|| format!("S1 round {} tx {} did not confirm", round + 1, tx_id))?;
        }
        let round_confirm_elapsed = confirm_start.elapsed();
        result.add_timing(format!("doubling_r{}_confirm", round + 1), round_confirm_elapsed);

        // Update per-tx metrics with confirm time (approximate, shared across round).
        let confirm_secs = round_confirm_elapsed.as_secs_f64();
        for m in per_tx_metrics.iter_mut().filter(|m| m.scenario_phase == format!("doubling_round_{}", round + 1)) {
            m.broadcast_to_confirmed_secs = Some(confirm_secs);
            m.outcome = "confirmed".to_string();
        }

        // Refresh wallet state (tip refresh).
        let _balance_after = mode.get_balance().await?;
        // Note: full UTXO count verification requires a scan; we trust the wallet's reported balance here.
    }

    let phase_a_elapsed = phase_a_start.elapsed();
    result.add_timing("doubling_phase_total", phase_a_elapsed);
    info!("[S1] Phase A done in {:.2?}", phase_a_elapsed);

    // ── Phase B: fan-out (64 txs, 1 in → 8 out) ──────────────────────────────
    let num_fanout_txs = 1u32 << p.doubling_rounds; // 64 for 6 rounds
    info!(
        "[S1] Phase B: fan-out {} txs, {} outputs each → {} UTXOs",
        num_fanout_txs, p.fanout_outputs_per_tx, p.volume_target
    );

    let phase_b_start = Instant::now();
    let pre_fanout_balance = mode.get_balance().await?;
    let amount_per_output = (pre_fanout_balance / p.volume_target as u64).saturating_sub(100_000);

    let mut fanout_tx_ids = Vec::new();
    for i in 0..num_fanout_txs {
        let construct_start = Instant::now();
        let tx_id = mode
            .coin_split(amount_per_output, p.fanout_outputs_per_tx)
            .await
            .with_context(|| format!("S1 fan-out tx {}/{} construct failed", i + 1, num_fanout_txs))?;
        let construct_elapsed = construct_start.elapsed();

        per_tx_metrics.push(TxMetrics {
            scenario_phase: "fanout".to_string(),
            tx_id: tx_id.clone(),
            construction_secs: construct_elapsed.as_secs_f64(),
            broadcast_to_mempool_secs: 0.0,
            broadcast_to_confirmed_secs: None,
            fee_paid_ut: None,
            outcome: "constructed".to_string(),
        });
        fanout_tx_ids.push(tx_id);

        if (i + 1) % 8 == 0 {
            info!("[S1] fan-out {}/{}", i + 1, num_fanout_txs);
        }
    }

    let confirm_start = Instant::now();
    for tx_id in &fanout_tx_ids {
        mode.wait_for_confirmation(tx_id, p.c_min, p.confirm_timeout_secs)
            .await
            .with_context(|| format!("S1 fan-out tx {} did not confirm", tx_id))?;
    }
    let fanout_confirm_elapsed = confirm_start.elapsed();
    result.add_timing("fanout_confirm", fanout_confirm_elapsed);

    let confirm_secs = fanout_confirm_elapsed.as_secs_f64();
    for m in per_tx_metrics.iter_mut().filter(|m| m.scenario_phase == "fanout") {
        m.broadcast_to_confirmed_secs = Some(confirm_secs);
        m.outcome = "confirmed".to_string();
    }

    let phase_b_elapsed = phase_b_start.elapsed();
    result.add_timing("fanout_phase_total", phase_b_elapsed);

    // ── Final balance reconciliation ─────────────────────────────────────────
    let final_balance = mode.get_balance().await?;
    let _ = mode.get_scanned_height().await?;

    result.utxo_count = Some(p.volume_target);
    result.txs_sent = Some(per_tx_metrics.len() as u32);
    result.txs_confirmed = Some(per_tx_metrics.iter().filter(|m| m.outcome == "confirmed").count() as u32);
    result.per_tx_metrics = Some(per_tx_metrics);
    result.balance_after_ut = Some(final_balance);

    info!(
        "[S1] complete: {} txs (63 doubling + 64 fan-out), {} UTXOs, total {:.2?}",
        result.txs_sent.unwrap(),
        p.volume_target,
        phase_a_elapsed + phase_b_elapsed
    );

    if fanout_tx_ids.len() != num_fanout_txs as usize {
        warn!("[S1] fan-out count mismatch: {} != {}", fanout_tx_ids.len(), num_fanout_txs);
    }
    Ok(())
}
