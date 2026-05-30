// S2 - Scan genesis (checkpoint 1): full chain rescan after S1 UTXO build-up.
//
// Purpose: Measure how long the wallet takes to rescan the entire chain from
// block 0 given the UTXO set built in S1.  This captures the read-path cost
// when there are many relevant outputs to detect and record.
//
// Run S1 before S2.  S2 does NOT re-run S1; it assumes the wallet already
// holds ~volume_target UTXOs in its local DB.
//
// Procedure:
//   1. Record current chain tip.
//   2. Trigger rescan from height 0 (genesis).
//   3. Wait until scanned_height reaches the recorded tip.
//   4. Record duration and block count.

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
    let tip = mode.get_scanned_height().await?;
    info!("[S2] genesis rescan from 0 to tip {}", tip);

    let scan_start = Instant::now();
    mode.rescan_from(0).await?;
    mode.wait_for_scan_height(tip, config.params.confirm_timeout_secs).await?;
    let elapsed = scan_start.elapsed();

    result.add_timing("scan", elapsed);
    result.scan_from_height = Some(0);
    result.blocks_scanned = Some(tip);

    info!("[S2] genesis rescan done: {} blocks in {:.2?}", tip, elapsed);
    Ok(())
}
