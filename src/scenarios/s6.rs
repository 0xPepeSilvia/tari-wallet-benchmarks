// S6 - Scan genesis (checkpoint 2): second full-chain rescan.
//
// Identical procedure to S2 but run after S4 and S5 have added more
// transactions to the chain.  Provides a second data point for genesis
// scan performance as the chain grows.
//
// This is explicitly called "checkpoint 2" in the bounty spec to distinguish
// it from the S2 measurement made before the S4/S5 transaction volume.

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
    info!("[S6] genesis rescan (checkpoint 2) from 0 to tip {}", tip);

    let scan_start = Instant::now();
    mode.rescan_from(0).await?;
    mode.wait_for_scan_height(tip, config.params.confirm_timeout_secs).await?;
    let elapsed = scan_start.elapsed();

    result.add_timing("scan", elapsed);
    result.scan_from_height = Some(0);
    result.blocks_scanned = Some(tip);

    info!("[S6] genesis rescan done: {} blocks in {:.2?}", tip, elapsed);
    Ok(())
}
