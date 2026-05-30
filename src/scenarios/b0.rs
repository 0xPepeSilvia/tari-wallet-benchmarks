// B0 - Baseline scan of an empty wallet from genesis.
//
// Purpose: Measure the raw cost of scanning the chain when there is nothing
// to detect.  This establishes the per-block overhead before any UTXOs exist.
//
// Procedure:
//   1. Trigger a rescan from height 0.
//   2. Wait until scanned_height reaches the chain tip.
//   3. Record scan time and block count.

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
    info!("[B0] starting genesis scan on empty wallet");

    let scan_start = Instant::now();

    // Trigger full rescan from genesis.
    mode.rescan_from(0).await?;

    // Poll until scanned_height stabilises (proxy: height > 0 and not moving for 2 polls).
    let timeout = config.params.confirm_timeout_secs;
    let poll = config.params.confirm_poll_secs;

    let mut last_height = 0u64;
    let mut stable_count = 0u32;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);

    loop {
        let h = mode.get_scanned_height().await?;
        info!("[B0] scanned_height = {}", h);

        if h > 0 && h == last_height {
            stable_count += 1;
            if stable_count >= 2 {
                break; // height hasn't moved for 2 consecutive polls - scan complete
            }
        } else {
            stable_count = 0;
        }
        last_height = h;

        if std::time::Instant::now() >= deadline {
            // Not a fatal error - record what we have.
            tracing::warn!("[B0] scan timed out at height {}", h);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(poll)).await;
    }

    let scan_duration = scan_start.elapsed();
    result.add_timing("scan", scan_duration);
    result.scan_from_height = Some(0);
    result.blocks_scanned = Some(last_height);

    info!(
        "[B0] genesis scan complete: {} blocks in {:.2?}",
        last_height, scan_duration
    );
    Ok(())
}
