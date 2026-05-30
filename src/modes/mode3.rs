// Mode 3 - Payment Processor (batch gRPC via minotari_console_wallet)
//
// Mode 3 uses the same wallet daemon as Mode 1 but exercises the batch
// `Transfer` call with multiple PaymentRecipient entries in a single request.
// This is the "payment processor" pattern: 1-to-M one-sided stealth transactions.
//
// Mode 3 re-uses Mode1Wallet's process management and gRPC client;
// the distinction is purely at the scenario layer where batch_send is called
// instead of individual send_to calls.

use crate::config::BenchmarkConfig;
use crate::modes::mode1::Mode1Wallet;
use crate::modes::WalletMode;
use anyhow::Result;
use async_trait::async_trait;

/// Mode 3 is a thin wrapper around Mode 1 that overrides the batch_send
/// semantics to use a true multi-recipient Transfer call.
pub struct Mode3Wallet {
    inner: Mode1Wallet,
}

impl Mode3Wallet {
    pub async fn new(config: &BenchmarkConfig, instance_id: &str) -> Result<Self> {
        // Mode 3 uses a separate data dir so it doesn't share state with Mode 1.
        Ok(Self {
            inner: Mode1Wallet::new(config, &format!("mode3-{}", instance_id)).await?,
        })
    }
}

#[async_trait]
impl WalletMode for Mode3Wallet {
    fn name(&self) -> &str { "Mode3/PaymentProcessor" }
    fn mode_number(&self) -> u8 { 3 }

    async fn start(&mut self) -> Result<()> { self.inner.start().await }
    async fn stop(&mut self) -> Result<()> { self.inner.stop().await }
    async fn get_address(&self) -> Result<String> { self.inner.get_address().await }
    async fn get_balance(&self) -> Result<u64> { self.inner.get_balance().await }
    async fn get_scanned_height(&self) -> Result<u64> { self.inner.get_scanned_height().await }

    async fn send_to(&self, address: &str, amount: u64) -> Result<String> {
        self.inner.send_to(address, amount).await
    }

    /// Mode 3's distinguishing feature: submit all recipients in one Transfer call.
    async fn batch_send(&self, recipients: &[(String, u64)]) -> Result<String> {
        self.inner.batch_send(recipients).await
    }

    async fn wait_for_scan_height(&self, target_height: u64, timeout_secs: u64) -> Result<()> {
        self.inner.wait_for_scan_height(target_height, timeout_secs).await
    }

    async fn rescan_from(&self, from_height: u64) -> Result<()> {
        self.inner.rescan_from(from_height).await
    }

    async fn coin_split(&self, amount_per_split: u64, count: u32) -> Result<String> {
        self.inner.coin_split(amount_per_split, count).await
    }

    async fn wait_for_confirmation(
        &self,
        tx_id: &str,
        min_confirmations: u32,
        timeout_secs: u64,
    ) -> Result<()> {
        self.inner.wait_for_confirmation(tx_id, min_confirmations, timeout_secs).await
    }
}
