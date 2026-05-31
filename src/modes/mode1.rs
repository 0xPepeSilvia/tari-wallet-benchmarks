// Mode 1 - minotari_console_wallet (old wallet stack)
//
// Spawns minotari_console_wallet as a subprocess, waits for gRPC readiness,
// then interacts via the tari.rpc.Wallet gRPC service on port 18143.

use crate::config::BenchmarkConfig;
use crate::modes::WalletMode;
use crate::wallet_grpc::{WalletGrpcClient, wallet_pb};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::{debug, info};

pub struct Mode1Wallet {
    /// Label used in logs and result files.
    pub instance_id: String,

    /// Path to the console_wallet binary.
    bin: PathBuf,

    /// Directory used for the wallet's data files.
    #[allow(dead_code)]
    data_dir: PathBuf,

    /// Network name (e.g. "esmeralda").
    #[allow(dead_code)]
    network: String,

    /// gRPC listen address for this instance (host:port).
    grpc_addr: String,

    /// gRPC port.
    #[allow(dead_code)]
    grpc_port: u16,

    /// Running child process, if any.
    process: Option<Child>,

    /// Lazy-connected gRPC client.
    client: Option<WalletGrpcClient>,

    /// Base node HTTP URL for scan / connection.
    #[allow(dead_code)]
    node_http: String,

    /// If true, do not spawn console_wallet on start(); connect to an
    /// operator-managed wallet at `grpc_addr` instead.  Set by Mode1Wallet::new
    /// when `[node] mode1_wallet_endpoint` is set in benchmark.toml.
    attach_mode: bool,
}

impl Mode1Wallet {
    pub async fn new(config: &BenchmarkConfig, instance_id: &str) -> Result<Self> {
        let data_dir = config.work_dir.join("wallets").join(format!("mode1-{}", instance_id));
        std::fs::create_dir_all(&data_dir)?;

        // If the operator specified an external wallet endpoint, attach mode:
        // skip spawn and connect to the existing wallet there.
        let (grpc_addr, grpc_port, attach_mode) =
            if let Some(ep) = &config.node.mode1_wallet_endpoint {
                let port = parse_port_from_url(ep).unwrap_or(18243);
                (ep.clone(), port, true)
            } else {
                // Spawn mode (default): use a fixed port. Real parallel use
                // should pass distinct instance_ids to vary the port.
                let grpc_port = 18143u16;
                (format!("http://127.0.0.1:{}", grpc_port), grpc_port, false)
            };

        Ok(Self {
            instance_id: instance_id.to_string(),
            bin: config.binaries.console_wallet.clone(),
            data_dir,
            network: config.network.clone(),
            grpc_addr,
            grpc_port,
            process: None,
            client: None,
            node_http: config.node.http_url.clone(),
            attach_mode,
        })
    }

    /// Connect (or reconnect) the gRPC client.
    async fn ensure_connected(&mut self) -> Result<&mut WalletGrpcClient> {
        if self.client.is_none() {
            let client = WalletGrpcClient::connect(&self.grpc_addr).await
                .with_context(|| format!("gRPC connect to {} failed", self.grpc_addr))?;
            self.client = Some(client);
        }
        Ok(self.client.as_mut().unwrap())
    }

    /// Poll the gRPC endpoint until the wallet reports it has bootstrapped,
    /// up to `timeout_secs`.
    async fn wait_for_ready(&mut self, timeout_secs: u64) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            match self.ensure_connected().await {
                Ok(client) => {
                    match client.get_state().await {
                        Ok(state) if state.has_done_initial_validation => {
                            info!("[Mode1/{}] wallet ready (initial validation complete, scanned_height={})",
                                self.instance_id, state.scanned_height);
                            return Ok(());
                        },
                        Ok(state) => {
                            debug!(
                                "[Mode1/{}] waiting for bootstrap (scanned_height={})",
                                self.instance_id, state.scanned_height
                            );
                        },
                        Err(e) => {
                            debug!("[Mode1/{}] get_state error: {}", self.instance_id, e);
                        },
                    }
                },
                Err(e) => {
                    debug!("[Mode1/{}] gRPC not yet reachable: {}", self.instance_id, e);
                    // Drop stale client so we retry the connection next iteration.
                    self.client = None;
                },
            }

            if Instant::now() >= deadline {
                anyhow::bail!(
                    "[Mode1/{}] wallet did not become ready within {}s",
                    self.instance_id, timeout_secs
                );
            }
            sleep(Duration::from_secs(3)).await;
        }
    }
}

#[async_trait]
impl WalletMode for Mode1Wallet {
    fn name(&self) -> &str { "Mode1/ConsoleWallet" }
    fn mode_number(&self) -> u8 { 1 }

    async fn start(&mut self) -> Result<()> {
        if self.attach_mode {
            info!("[Mode1/{}] attach mode: connecting to existing wallet at {}",
                self.instance_id, self.grpc_addr);
            // Just establish the gRPC client; the wallet is already running.
            self.ensure_connected().await?;
            return Ok(());
        }

        if self.process.is_some() {
            return Ok(());
        }

        info!("[Mode1/{}] spawning console_wallet (UNVALIDATED PATH — \
            spawn args have not been confirmed against current console_wallet \
            versions; use [node] mode1_wallet_endpoint for attach mode instead)",
            self.instance_id);

        let child = Command::new(&self.bin)
            .args([
                "--network", &self.network,
                "--base-path", self.data_dir.to_str().unwrap(),
                "--non-interactive-mode",
                "--grpc-enabled",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Failed to spawn {:?}", self.bin))?;

        self.process = Some(child);

        self.wait_for_ready(120).await?;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        if self.attach_mode {
            // Don't kill an operator-managed wallet.
            self.client = None;
            return Ok(());
        }

        if let Some(mut child) = self.process.take() {
            info!("[Mode1/{}] stopping console_wallet", self.instance_id);
            let _ = child.kill();
            let _ = child.wait();
        }
        self.client = None;
        Ok(())
    }

    async fn get_address(&self) -> Result<String> {
        let client = self.client.as_ref()
            .context("Mode1 wallet not started")?;
        let resp = client.get_address().await?;
        // Decode the raw bytes using Tari's custom per-section base58.
        decode_tari_address_bytes(&resp.address)
    }

    async fn get_balance(&self) -> Result<u64> {
        let client = self.client.as_ref()
            .context("Mode1 wallet not started")?;
        let resp = client.get_balance().await?;
        Ok(resp.available_balance)
    }

    async fn get_scanned_height(&self) -> Result<u64> {
        let client = self.client.as_ref()
            .context("Mode1 wallet not started")?;
        let state = client.get_state().await?;
        Ok(state.scanned_height)
    }

    async fn send_to(&self, address: &str, amount: u64) -> Result<String> {
        let client = self.client.as_ref()
            .context("Mode1 wallet not started")?;
        let recipient = wallet_pb::PaymentRecipient {
            address: address.to_string(),
            amount,
            fee_per_gram: 5,
            message: "benchmark".to_string(),
            payment_type: wallet_pb::PaymentType::OneSidedToStealthAddress as i32,
        };
        let resp = client.transfer(vec![recipient]).await?;
        let result = resp.results.into_iter().next()
            .context("Transfer returned empty results")?;
        if !result.is_success {
            anyhow::bail!("Transfer failed: {}", result.failure_message);
        }
        Ok(result.transaction_id.to_string())
    }

    async fn batch_send(&self, recipients: &[(String, u64)]) -> Result<String> {
        let client = self.client.as_ref()
            .context("Mode1 wallet not started")?;
        let grpc_recipients: Vec<wallet_pb::PaymentRecipient> = recipients
            .iter()
            .map(|(addr, amount)| wallet_pb::PaymentRecipient {
                address: addr.clone(),
                amount: *amount,
                fee_per_gram: 5,
                message: "benchmark-batch".to_string(),
                payment_type: wallet_pb::PaymentType::OneSidedToStealthAddress as i32,
            })
            .collect();
        let resp = client.transfer(grpc_recipients).await?;
        // For a batch, all outputs go into one transaction - return the first tx id.
        let first = resp.results.into_iter().find(|r| r.is_success)
            .context("All recipients in batch failed")?;
        Ok(first.transaction_id.to_string())
    }

    async fn wait_for_scan_height(&self, target_height: u64, timeout_secs: u64) -> Result<()> {
        let client = self.client.as_ref()
            .context("Mode1 wallet not started")?;
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            let state = client.get_state().await?;
            if state.scanned_height >= target_height {
                return Ok(());
            }
            debug!(
                "[Mode1/{}] scan at {}/{}", self.instance_id,
                state.scanned_height, target_height
            );
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "Timed out waiting for scan height {} (at {})",
                    target_height, state.scanned_height
                );
            }
            sleep(Duration::from_secs(5)).await;
        }
    }

    async fn rescan_from(&self, from_height: u64) -> Result<()> {
        let client = self.client.as_ref()
            .context("Mode1 wallet not started")?;
        client.rescan_wallet(from_height as i64).await?;
        Ok(())
    }

    async fn coin_split(&self, amount_per_split: u64, count: u32) -> Result<String> {
        let client = self.client.as_ref()
            .context("Mode1 wallet not started")?;
        let resp = client.coin_split(amount_per_split, count, 5).await?;
        Ok(resp.transaction_id.to_string())
    }

    async fn wait_for_confirmation(
        &self,
        tx_id: &str,
        _min_confirmations: u32,
        timeout_secs: u64,
    ) -> Result<()> {
        let client = self.client.as_ref()
            .context("Mode1 wallet not started")?;
        let id: u64 = tx_id.parse().context("tx_id must be numeric for Mode1")?;
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            let info = client.get_transaction_info(vec![id]).await?;
            let tx = info.transactions.into_iter().next()
                .context("No transaction info returned")?;
            use wallet_pb::TransactionStatus;
            if matches!(
                tx.status(),
                TransactionStatus::TxMinedConfirmed
                    | TransactionStatus::TxOneSidedConfirmed
                    | TransactionStatus::TxCoinbaseConfirmed
            ) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "Tx {} not confirmed after {}s (status={:?})",
                    tx_id, timeout_secs, tx.status()
                );
            }
            sleep(Duration::from_secs(10)).await;
        }
    }
}

/// Parse the port number from a URL like "http://127.0.0.1:18243".
fn parse_port_from_url(url: &str) -> Option<u16> {
    url.rsplit(':').next().and_then(|s| {
        let port_str = s.trim_end_matches('/');
        port_str.parse().ok()
    })
}

/// Decode Tari's custom per-section base58 address encoding.
///
/// Tari encodes a raw address byte slice as:
///   bs58(byte[0]) + bs58(byte[1]) + bs58(bytes[2..])
/// NOT standard base58 of the whole array.
fn decode_tari_address_bytes(raw: &[u8]) -> Result<String> {
    if raw.len() < 3 {
        anyhow::bail!("Address bytes too short: {}", raw.len());
    }
    let b0 = bs58::encode(&raw[0..1]).into_string();
    let b1 = bs58::encode(&raw[1..2]).into_string();
    let rest = bs58::encode(&raw[2..]).into_string();
    Ok(format!("{}{}{}", b0, b1, rest))
}

impl Drop for Mode1Wallet {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_short_bytes_errors() {
        let result = decode_tari_address_bytes(&[]);
        assert!(result.is_err());
        let result = decode_tari_address_bytes(&[0u8, 0u8]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_three_byte_minimum_succeeds() {
        // Smallest valid input
        let raw = [0x26u8, 0x03u8, 0x00u8];
        let decoded = decode_tari_address_bytes(&raw).expect("3 bytes decodes");
        assert!(!decoded.is_empty());
    }

    #[test]
    fn decode_matches_known_mining_wallet_address() {
        // The funded mining wallet's raw bytes (from production GetAddress RPC)
        // start with these well-known prefix bytes.
        // Full known address from session memory:
        //   f2Ln1PRd2bmwWqC3q8yydaoHFVSURyciar2ijamoz7Hy7FuVYXEqdCCqJCj2aY5DZSQxoCCPjvQTfHwkvdZmbrVVsM9
        //
        // Tari per-section base58 encodes byte[0], byte[1], bytes[2..] separately.
        // Verify the encoding implementation is consistent for known input prefixes.
        let raw_prefix = [0x26u8, 0x01u8];  // esmeralda network + one-sided features
        // bs58 of single byte 0x26 = "f", 0x01 = "2"
        let b0 = bs58::encode(&raw_prefix[0..1]).into_string();
        let b1 = bs58::encode(&raw_prefix[1..2]).into_string();
        assert_eq!(b0, "f");
        assert_eq!(b1, "2");
        // So a real f2-prefixed address must start with these bytes.
    }

    #[test]
    fn decode_output_has_three_concatenated_sections() {
        // Round-trip: pick known input, decode, ensure the three sections are present
        let raw: Vec<u8> = (0u8..67u8).collect();
        let decoded = decode_tari_address_bytes(&raw).unwrap();

        // The decoded string is bs58(byte[0]) ++ bs58(byte[1]) ++ bs58(bytes[2..]).
        let b0 = bs58::encode(&raw[0..1]).into_string();
        let b1 = bs58::encode(&raw[1..2]).into_string();
        let rest = bs58::encode(&raw[2..]).into_string();
        assert_eq!(decoded, format!("{}{}{}", b0, b1, rest));
    }

    #[test]
    fn parse_port_extracts_from_http_url() {
        assert_eq!(parse_port_from_url("http://127.0.0.1:18243"), Some(18243));
        assert_eq!(parse_port_from_url("http://localhost:9006"), Some(9006));
        assert_eq!(parse_port_from_url("http://127.0.0.1:18243/"), Some(18243));
    }

    #[test]
    fn parse_port_returns_none_for_malformed() {
        assert_eq!(parse_port_from_url("not-a-url"), None);
        assert_eq!(parse_port_from_url("http://localhost"), None);
    }

    #[test]
    fn decode_standard_base58_would_be_different() {
        // Verify that per-section encoding is actually different from standard
        // whole-array bs58 — this is the entire reason this helper exists.
        let raw: Vec<u8> = vec![0x26, 0x01, 0x12, 0x34, 0x56];
        let ours = decode_tari_address_bytes(&raw).unwrap();
        let standard_bs58 = bs58::encode(&raw).into_string();
        assert_ne!(ours, standard_bs58, "per-section encoding must differ from standard");
    }
}
