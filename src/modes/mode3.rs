// Mode 3 - Payment Processor.
//
// SWvheerden clarification (2026-05-28 on tari-project/wallet-benchmarks#1):
//   "There is some confusion it seems on the payment processor, its this
//    application: https://github.com/tari-project/minotari_payment_processor"
//
// So Mode 3 is NOT just batch Transfer gRPC on console_wallet - it is the
// `minotari_payment_processor` service which runs 5 background workers:
//   batch_creator → unsigned_tx_creator → transaction_signer → broadcaster → confirmation_checker
//
// The processor consumes payment instructions from an upstream PAYMENT_RECEIVER
// HTTP API and pushes signed transactions onto the chain via a console_wallet
// subprocess for signing.
//
// Architectural implications for the harness (raised by roadhero on 2026-05-29,
// awaiting SW's response):
//   - Scan scenarios (B0, S2, S3, S6, S7) are NOT meaningful for Mode 3 since
//     the processor does not own a scanning wallet.
//   - S4/S5 concurrent + batch are the relevant measurements.
//   - The harness must stand up a stub PAYMENT_RECEIVER HTTP service to feed
//     payment instructions to the processor.
//
// This module spawns the payment processor binary and drives it via its HTTP
// API.  The PAYMENT_RECEIVER stub is provided in `payment_receiver_stub.rs`.

use crate::config::BenchmarkConfig;
use crate::modes::WalletMode;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use tracing::{info, warn};

pub struct Mode3Wallet {
    pub instance_id: String,
    bin: Option<PathBuf>,
    data_dir: PathBuf,
    process: Option<Child>,
    /// HTTP port the processor listens on for API + Swagger.
    api_port: u16,
    /// PAYMENT_RECEIVER stub URL (provided by harness).
    payment_receiver_url: String,
    /// Base node URL.
    base_node_url: String,
}

impl Mode3Wallet {
    pub async fn new(config: &BenchmarkConfig, instance_id: &str) -> Result<Self> {
        let data_dir = config.work_dir.join("wallets").join(format!("mode3-{}", instance_id));
        std::fs::create_dir_all(&data_dir)?;

        Ok(Self {
            instance_id: instance_id.to_string(),
            bin: config.binaries.payment_processor.clone(),
            data_dir,
            process: None,
            api_port: 9145,
            payment_receiver_url: std::env::var("TARI_BENCH_PAYMENT_RECEIVER_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:9146".to_string()),
            base_node_url: config.node.http_url.clone(),
        })
    }
}

#[async_trait]
impl WalletMode for Mode3Wallet {
    fn name(&self) -> &str { "Mode3/PaymentProcessor" }
    fn mode_number(&self) -> u8 { 3 }

    async fn start(&mut self) -> Result<()> {
        let bin = self.bin.as_ref().context(
            "Mode 3 requires binaries.payment_processor to be set in benchmark.toml. \
             Build minotari_payment_processor from https://github.com/tari-project/minotari_payment_processor \
             and set the path."
        )?;

        if self.process.is_some() {
            return Ok(());
        }

        info!("[Mode3/{}] spawning minotari_payment_processor", self.instance_id);
        warn!(
            "[Mode3/{}] PAYMENT_RECEIVER stub at {} - ensure it is running",
            self.instance_id, self.payment_receiver_url
        );

        let db_url = format!("sqlite://{}/payment_processor.db", self.data_dir.display());
        let child = Command::new(bin)
            .env("DATABASE_URL", &db_url)
            .env("PAYMENT_RECEIVER", &self.payment_receiver_url)
            .env("BASE_NODE", &self.base_node_url)
            .env("LISTEN_PORT", self.api_port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Failed to spawn {:?}", bin))?;

        self.process = Some(child);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }

    // ── Scan scenarios are not meaningful for Mode 3 ──────────────────────────
    // The processor does not own a scanning wallet of its own; it relies on the
    // console_wallet subprocess for signing only.  Surfacing these as runtime
    // errors lets the scenario runner record them as Skipped with reason.

    async fn get_address(&self) -> Result<String> {
        anyhow::bail!("Mode 3 has no single receiving address (it pays multiple recipients per batch). \
                       Configure recipients via the PAYMENT_RECEIVER stub.");
    }

    async fn get_balance(&self) -> Result<u64> {
        anyhow::bail!("Mode 3 has no balance concept at the harness level - query the upstream \
                       PAYMENT_RECEIVER for funds available to draw on.");
    }

    async fn get_scanned_height(&self) -> Result<u64> {
        anyhow::bail!("Mode 3 does not scan - skip B0/S2/S3/S6/S7 for this mode.");
    }

    async fn send_to(&self, _address: &str, _amount: u64) -> Result<String> {
        anyhow::bail!("Mode 3 does not support direct send - submit payment instructions via the \
                       PAYMENT_RECEIVER API instead.");
    }

    async fn batch_send(&self, _recipients: &[(String, u64)]) -> Result<String> {
        // TODO: POST batch payment instructions to the PAYMENT_RECEIVER stub,
        //       which the processor will then poll and convert into batch txs.
        anyhow::bail!("Mode 3 batch_send: not yet implemented. Awaiting SW's response to \
                       roadhero's 2026-05-29 question about the PAYMENT_RECEIVER upstream.");
    }

    async fn wait_for_scan_height(&self, _target: u64, _timeout: u64) -> Result<()> {
        anyhow::bail!("Mode 3 does not scan.");
    }

    async fn rescan_from(&self, _from: u64) -> Result<()> {
        anyhow::bail!("Mode 3 does not scan.");
    }

    async fn coin_split(&self, _amount: u64, _count: u32) -> Result<String> {
        anyhow::bail!("Mode 3 does not support coin_split - it is a service that pays recipients.");
    }

    async fn wait_for_confirmation(&self, _tx_id: &str, _min: u32, _timeout: u64) -> Result<()> {
        // TODO: query the processor's HTTP API for tx confirmation state.
        anyhow::bail!("Mode 3 wait_for_confirmation: not yet implemented.");
    }
}

impl Drop for Mode3Wallet {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
        }
    }
}
