// Wallet mode implementations.
//
// Each mode wraps a different wallet stack and exposes the same `WalletMode`
// trait so the scenario layer can be wallet-agnostic.

pub mod mode1;
pub mod mode2;
pub mod mode3;

use crate::config::BenchmarkConfig;
use anyhow::Result;
use async_trait::async_trait;

// ─────────────────────────────────────────────────────────────────────────────
// WalletMode trait
// ─────────────────────────────────────────────────────────────────────────────

/// Capabilities a wallet mode must expose to the scenario runner.
///
/// All amounts are in µT.  All operations are async and cancellable via the
/// tokio runtime cancellation (callers should wrap with timeout where needed).
#[async_trait]
pub trait WalletMode: Send + Sync {
    /// Human-readable name (e.g. "Mode1/ConsoleWallet").
    fn name(&self) -> &str;

    /// The mode number (1, 2, or 3).
    fn mode_number(&self) -> u8;

    /// Start the wallet process / service.  Idempotent if already running.
    async fn start(&mut self) -> Result<()>;

    /// Stop the wallet process cleanly.
    async fn stop(&mut self) -> Result<()>;

    /// Return the wallet's one-sided receiving address as a base-58 string.
    async fn get_address(&self) -> Result<String>;

    /// Return confirmed available balance in µT.
    async fn get_balance(&self) -> Result<u64>;

    /// Return the chain height the wallet has scanned up to.
    async fn get_scanned_height(&self) -> Result<u64>;

    /// Send `amount` µT to `address` (one-sided stealth).
    /// Returns a transaction identifier (opaque string).
    async fn send_to(&self, address: &str, amount: u64) -> Result<String>;

    /// Send `amount` µT to each address in `recipients` in a single batched
    /// transaction (used by Mode 3 / S5 batch path).
    /// Returns a single transaction identifier.
    async fn batch_send(&self, recipients: &[(String, u64)]) -> Result<String>;

    /// Block until the wallet's scanned_height reaches at least `target_height`,
    /// or until `timeout_secs` elapses.
    async fn wait_for_scan_height(
        &self,
        target_height: u64,
        timeout_secs: u64,
    ) -> Result<()>;

    /// Trigger a full rescan from `from_height`.
    async fn rescan_from(&self, from_height: u64) -> Result<()>;

    /// Split the wallet's largest UTXO into `count` outputs of `amount_per_split` µT each.
    async fn coin_split(&self, amount_per_split: u64, count: u32) -> Result<String>;

    /// Wait for `tx_id` to reach Confirmed status (at least `min_confirmations`).
    async fn wait_for_confirmation(
        &self,
        tx_id: &str,
        min_confirmations: u32,
        timeout_secs: u64,
    ) -> Result<()>;
}

// ─────────────────────────────────────────────────────────────────────────────
// Factory
// ─────────────────────────────────────────────────────────────────────────────

/// Construct a wallet mode instance from config.
/// Returns a boxed `WalletMode` ready for `start()`.
pub async fn build_mode(
    mode_number: u8,
    config: &BenchmarkConfig,
    instance_id: &str,
) -> Result<Box<dyn WalletMode>> {
    match mode_number {
        1 => Ok(Box::new(mode1::Mode1Wallet::new(config, instance_id).await?)),
        2 => Ok(Box::new(mode2::Mode2Wallet::new(config, instance_id).await?)),
        3 => Ok(Box::new(mode3::Mode3Wallet::new(config, instance_id).await?)),
        n => anyhow::bail!("Unknown mode number {}", n),
    }
}
