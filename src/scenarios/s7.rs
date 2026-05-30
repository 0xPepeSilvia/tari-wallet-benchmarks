// S7 - Scan from birthday (checkpoint 2): second birthday rescan.
//
// Identical procedure to S3 but run after S4/S5 have added more chain history.
// Provides a second data point for birthday-based scan performance.

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

    let birthday: u64 = std::env::var("TARI_BENCH_BIRTHDAY_HEIGHT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| tip.saturating_sub(1000));

    info!(
        "[S7] birthday rescan (checkpoint 2) from {} to tip {}  ({} blocks)",
        birthday, tip, tip - birthday
    );

    let scan_start = Instant::now();
    mode.rescan_from(birthday).await?;
    mode.wait_for_scan_height(tip, config.params.confirm_timeout_secs).await?;
    let elapsed = scan_start.elapsed();

    result.add_timing("scan", elapsed);
    result.scan_from_height = Some(birthday);
    result.blocks_scanned = Some(tip - birthday);

    info!("[S7] birthday rescan done: {} blocks in {:.2?}", tip - birthday, elapsed);
    Ok(())
}
