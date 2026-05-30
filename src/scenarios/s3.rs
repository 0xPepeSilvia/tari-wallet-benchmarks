// S3 - Scan from wallet birthday.
//
// Purpose: Measure rescan cost when starting from the wallet's creation height
// rather than genesis.  In practice wallets should always know their birthday
// and use it to skip irrelevant history.
//
// "Birthday" for this harness = the block height at which S0 (funding) completed.
// We record this height in the ScenarioResult so the report can compare it with
// the genesis-scan results from S2/S6.
//
// Procedure:
//   1. Read the birthday height from the harness state (set during S0).
//      If not available, use scanned_height - (chain_tip - birthday_estimate)
//      or fall back to 1000 blocks before current tip.
//   2. Record current chain tip.
//   3. Trigger rescan from birthday height.
//   4. Wait until scanned_height reaches the tip.
//   5. Record timing.

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

    // Resolve birthday height.
    let birthday: u64 = std::env::var("TARI_BENCH_BIRTHDAY_HEIGHT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            // Fallback: assume wallet was created ~1000 blocks before current tip.
            tip.saturating_sub(1000)
        });

    info!(
        "[S3] birthday rescan from {} to tip {}  ({} blocks)",
        birthday, tip, tip - birthday
    );

    let scan_start = Instant::now();
    mode.rescan_from(birthday).await?;
    mode.wait_for_scan_height(tip, config.params.confirm_timeout_secs).await?;
    let elapsed = scan_start.elapsed();

    result.add_timing("scan", elapsed);
    result.scan_from_height = Some(birthday);
    result.blocks_scanned = Some(tip - birthday);

    info!("[S3] birthday rescan done: {} blocks in {:.2?}", tip - birthday, elapsed);
    Ok(())
}
