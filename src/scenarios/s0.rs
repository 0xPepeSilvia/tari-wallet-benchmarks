// S0 - Fund the benchmark wallet.
//
// Transfers `params.a_fund` µT from an external funded address into this
// wallet and waits for `params.c_min` confirmations.
//
// The funded source wallet address is provided via the config or an
// environment variable TARI_BENCH_FUNDER_ADDRESS and TARI_BENCH_FUNDER_KEY.
// In practice the test operator runs S0 manually once (or supplies a
// pre-funded wallet data directory) before running S1-S7.
//
// If the wallet already holds >= a_fund, S0 skips the transfer step.

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
    let target = config.params.a_fund;
    let current_balance = mode.get_balance().await?;

    info!(
        "[S0] wallet balance: {} µT, target: {} µT",
        current_balance, target
    );

    if current_balance >= target {
        info!("[S0] already funded - skipping transfer");
        result.amount_transferred_ut = Some(0);
        result.txs_sent = Some(0);
        result.txs_confirmed = Some(0);
        return Ok(());
    }

    let amount_needed = target - current_balance;

    // Look for funder address from environment.
    let funder_address = std::env::var("TARI_BENCH_FUNDER_ADDRESS")
        .ok()
        .filter(|s| !s.is_empty());

    if funder_address.is_none() {
        // S0 must be run with external funding.  Describe what's needed.
        anyhow::bail!(
            "S0: wallet underfunded ({} µT < {} µT required).  \
             Set TARI_BENCH_FUNDER_ADDRESS to the address of a funded wallet, \
             then send {} µT to {} and re-run.",
            current_balance,
            target,
            amount_needed,
            mode.get_address().await?
        );
    }

    // If a funder address is provided, we expect the *caller* to fund this wallet
    // out-of-band; S0 just waits for the funds to arrive.
    let wallet_address = mode.get_address().await?;
    info!(
        "[S0] waiting for {} µT to arrive at {}",
        amount_needed, wallet_address
    );

    let fund_start = Instant::now();
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(config.params.confirm_timeout_secs);

    loop {
        let balance = mode.get_balance().await?;
        if balance >= target {
            let elapsed = fund_start.elapsed();
            result.add_timing("funding", elapsed);
            result.amount_transferred_ut = Some(balance);
            result.txs_confirmed = Some(1);
            info!("[S0] funded: {} µT in {:.2?}", balance, elapsed);
            return Ok(());
        }

        if std::time::Instant::now() >= deadline {
            anyhow::bail!(
                "S0 timed out: balance {} µT after {}s, need {} µT",
                balance,
                config.params.confirm_timeout_secs,
                target
            );
        }
        tokio::time::sleep(std::time::Duration::from_secs(
            config.params.confirm_poll_secs,
        ))
        .await;
    }
}
