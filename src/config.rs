// Harness configuration - all parameters from the bounty spec in one place.
//
// Loaded from `benchmark.toml` at the path specified by --config (default: ./benchmark.toml).
// Every field has a documented default so you can run with a minimal config file.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level config struct - deserialised from benchmark.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkConfig {
    /// Network the harness targets (e.g. "esmeralda", "nextnet", "mainnet").
    #[serde(default = "default_network")]
    pub network: String,

    /// Path where the harness stores its own state, temp files and result profiles.
    #[serde(default = "default_work_dir")]
    pub work_dir: PathBuf,

    /// Paths to the Tari binaries on this machine.
    pub binaries: BinariesConfig,

    /// Node connectivity.
    pub node: NodeConfig,

    /// Scenario tuning - corresponds 1:1 to the bounty spec parameter table.
    #[serde(default)]
    pub params: ScenarioParams,

    /// Which wallet modes to run (subset of [1, 2, 3]).
    /// Defaults to all three.
    #[serde(default = "all_modes")]
    pub modes: Vec<u8>,

    /// Which scenarios to run (subset of ["B0","S0","S1","S2","S3","S4","S5","S6","S7"]).
    /// Defaults to all nine.
    #[serde(default = "all_scenarios")]
    pub scenarios: Vec<String>,
}

/// Absolute paths to the executables used by the harness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinariesConfig {
    /// `minotari_console_wallet` binary (Mode 1 and funding).
    pub console_wallet: PathBuf,

    /// `minotari_node` binary (spawned as local regtest node if `node.spawn_local = true`).
    pub base_node: PathBuf,

    /// `minotari` (minotari-cli) binary (Mode 2 unsigned-transaction creation).
    pub minotari_cli: PathBuf,

    /// `minotari_payment_processor` binary (Mode 3).
    /// See https://github.com/tari-project/minotari_payment_processor.
    /// Optional - if not set, Mode 3 will be skipped with a clear error.
    #[serde(default)]
    pub payment_processor: Option<PathBuf>,
}

/// Base-node connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// JSON-RPC endpoint for transaction submission (Mode 2/3).
    /// e.g. "http://127.0.0.1:18142"
    #[serde(default = "default_node_http")]
    pub http_url: String,

    /// gRPC endpoint for the base node if needed (not used directly by the harness yet).
    #[serde(default = "default_node_grpc")]
    pub grpc_url: String,

    /// If true, the harness spawns and manages a local `minotari_node` instance.
    /// Set to false to connect to an already-running node.
    #[serde(default)]
    pub spawn_local: bool,
}

/// All scenario tuning knobs from the bounty spec.
/// Defaults match the spec defaults exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioParams {
    // ── Funding (S0) ────────────────────────────────────────────────────────
    /// Amount (µT) to transfer into each mode's wallet during S0.
    /// Spec default: 10_000_000_000 µT (10,000 tXTM).
    #[serde(default = "default_a_fund")]
    pub a_fund: u64,

    /// Minimum confirmations before S0 funding is considered complete.
    #[serde(default = "default_c_min")]
    pub c_min: u32,

    // ── UTXO build-up (S1) ───────────────────────────────────────────────────
    /// Target UTXO count after the S1 fan-out phase.
    /// Spec: 512.
    #[serde(default = "default_volume_target")]
    pub volume_target: u32,

    /// Number of serial doubling rounds (1-in-2-out) before fan-out.
    /// Spec: 6 rounds → 64 inputs.
    #[serde(default = "default_doubling_rounds")]
    pub doubling_rounds: u32,

    /// Number of outputs per fan-out transaction.
    /// Spec: 8 outputs/tx.  64 inputs × 8 outputs = 512.
    #[serde(default = "default_fanout_outputs_per_tx")]
    pub fanout_outputs_per_tx: u32,

    // ── Concurrent construction (S4) ────────────────────────────────────────
    /// Concurrent worker batch sizes to probe.
    /// Spec: [8, 16, 32, 64, 128].
    #[serde(default = "default_s4_concurrency_levels")]
    pub s4_concurrency_levels: Vec<u32>,

    /// Wall-clock budget per concurrency level (seconds).
    /// Spec: 900 s (15 min).
    #[serde(default = "default_s4_budget_secs")]
    pub s4_budget_secs: u64,

    // ── Payment-processor batch (S5) ────────────────────────────────────────
    /// Batch size for the batched run.
    /// Spec: M = 100 recipients per batch transaction.
    #[serde(default = "default_s5_m")]
    pub s5_m: u32,

    /// Number of individual transactions for the individual run.
    /// Spec: K = 10 individual transactions.
    #[serde(default = "default_s5_k")]
    pub s5_k: u32,

    // ── Fee rate ─────────────────────────────────────────────────────────────
    /// Fee per gram (µT/gram) used for all constructed transactions.
    #[serde(default = "default_fee_per_gram")]
    pub fee_per_gram: u64,

    // ── Confirmation polling ─────────────────────────────────────────────────
    /// How long to poll for confirmation before timing out (seconds).
    #[serde(default = "default_confirm_timeout_secs")]
    pub confirm_timeout_secs: u64,

    /// Polling interval while waiting for confirmations (seconds).
    #[serde(default = "default_confirm_poll_secs")]
    pub confirm_poll_secs: u64,
}

impl Default for ScenarioParams {
    fn default() -> Self {
        Self {
            a_fund: default_a_fund(),
            c_min: default_c_min(),
            volume_target: default_volume_target(),
            doubling_rounds: default_doubling_rounds(),
            fanout_outputs_per_tx: default_fanout_outputs_per_tx(),
            s4_concurrency_levels: default_s4_concurrency_levels(),
            s4_budget_secs: default_s4_budget_secs(),
            s5_m: default_s5_m(),
            s5_k: default_s5_k(),
            fee_per_gram: default_fee_per_gram(),
            confirm_timeout_secs: default_confirm_timeout_secs(),
            confirm_poll_secs: default_confirm_poll_secs(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Default value functions (required by serde default attributes)
// ─────────────────────────────────────────────────────────────────────────────

fn default_network() -> String { "esmeralda".to_string() }
fn default_work_dir() -> PathBuf { PathBuf::from("./benchmark-work") }
fn default_node_http() -> String { "http://127.0.0.1:18142".to_string() }
fn default_node_grpc() -> String { "http://127.0.0.1:18142".to_string() }
fn all_modes() -> Vec<u8> { vec![1, 2, 3] }
fn all_scenarios() -> Vec<String> {
    ["B0","S0","S1","S2","S3","S4","S5","S6","S7"]
        .iter().map(|s| s.to_string()).collect()
}

fn default_a_fund() -> u64 { 10_000_000_000 }   // 10,000 tXTM in µT
fn default_c_min() -> u32 { 3 }
fn default_volume_target() -> u32 { 512 }
fn default_doubling_rounds() -> u32 { 6 }
fn default_fanout_outputs_per_tx() -> u32 { 8 }
fn default_s4_concurrency_levels() -> Vec<u32> { vec![8, 16, 32, 64, 128] }
fn default_s4_budget_secs() -> u64 { 900 }
fn default_s5_m() -> u32 { 100 }
fn default_s5_k() -> u32 { 10 }
fn default_fee_per_gram() -> u64 { 5 }
fn default_confirm_timeout_secs() -> u64 { 7200 }  // 2 hours
fn default_confirm_poll_secs() -> u64 { 10 }

// ─────────────────────────────────────────────────────────────────────────────
// Loader
// ─────────────────────────────────────────────────────────────────────────────

impl BenchmarkConfig {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Cannot read config {:?}: {}", path, e))?;
        let cfg: Self = toml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("Config parse error in {:?}: {}", path, e))?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.modes.is_empty() {
            anyhow::bail!("config.modes must have at least one entry");
        }
        for m in &self.modes {
            if ![1u8, 2, 3].contains(m) {
                anyhow::bail!("config.modes contains unknown mode {}; valid values are 1, 2, 3", m);
            }
        }
        let valid_scenarios = ["B0","S0","S1","S2","S3","S4","S5","S6","S7"];
        for s in &self.scenarios {
            if !valid_scenarios.contains(&s.as_str()) {
                anyhow::bail!(
                    "config.scenarios contains unknown scenario '{}'; valid values are {:?}",
                    s, valid_scenarios
                );
            }
        }
        if self.params.doubling_rounds > 10 {
            anyhow::bail!("params.doubling_rounds > 10 is unreasonably large");
        }
        if self.params.a_fund == 0 {
            anyhow::bail!("params.a_fund must be > 0");
        }
        Ok(())
    }
}
