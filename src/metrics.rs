// Result types emitted by the harness.
//
// Every scenario × mode run produces a `ScenarioResult` which is accumulated
// into a `BenchmarkReport` and serialised to JSON at the end.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

// ─────────────────────────────────────────────────────────────────────────────
// Top-level report
// ─────────────────────────────────────────────────────────────────────────────

/// Complete output of a harness run.  Spec-mandated fields:
/// - Hardware / environment disclosure
/// - Pinned versions of all binaries
/// - All configuration parameter values used
/// - Per-scenario, per-mode metrics
/// - Computed deltas
#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub run_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub network: String,

    /// SHA-256 of the benchmark.toml used (for reproducibility).
    pub config_sha256: String,

    /// Hardware + OS disclosure (CPU model, RAM, disk type, OS).
    pub environment: EnvironmentInfo,

    /// Pinned binary versions captured at run start.
    pub binary_versions: BinaryVersions,

    /// Full snapshot of the harness config used for this run.
    pub config_snapshot: serde_json::Value,

    /// Per-scenario × mode results.
    pub results: Vec<ScenarioResult>,

    /// Computed cross-scenario deltas (S2-B0, S6-S2, S5 multiplier, etc).
    pub deltas: HashMap<String, f64>,
}

impl BenchmarkReport {
    pub fn new(
        run_id: String,
        network: String,
        config_sha256: String,
        environment: EnvironmentInfo,
        binary_versions: BinaryVersions,
        config_snapshot: serde_json::Value,
    ) -> Self {
        Self {
            run_id,
            started_at: Utc::now(),
            finished_at: None,
            network,
            config_sha256,
            environment,
            binary_versions,
            config_snapshot,
            results: Vec::new(),
            deltas: HashMap::new(),
        }
    }

    pub fn push(&mut self, r: ScenarioResult) {
        self.results.push(r);
    }

    pub fn finish(&mut self) {
        self.compute_deltas();
        self.finished_at = Some(Utc::now());
    }

    /// Compute spec-mandated deltas: T_scan(S2)-T_scan(B0), T_scan(S6)-T_scan(S2),
    /// T_scan(S6)/T_scan(B0), S5 throughput multiplier.
    fn compute_deltas(&mut self) {
        for mode in [1u8, 2, 3] {
            let find = |scenario: &str| -> Option<f64> {
                self.results
                    .iter()
                    .find(|r| r.mode == mode && r.scenario == scenario && r.status == ScenarioStatus::Passed)
                    .and_then(|r| r.timings.get("scan").copied())
            };

            if let (Some(s2), Some(b0)) = (find("S2"), find("B0")) {
                self.deltas.insert(format!("mode{}_S2_minus_B0_secs", mode), s2 - b0);
            }
            if let (Some(s6), Some(s2)) = (find("S6"), find("S2")) {
                self.deltas.insert(format!("mode{}_S6_minus_S2_secs", mode), s6 - s2);
            }
            if let (Some(s6), Some(b0)) = (find("S6"), find("B0")) {
                if b0 > 0.0 {
                    self.deltas.insert(format!("mode{}_S6_over_B0_ratio", mode), s6 / b0);
                }
            }

            // S5 throughput multiplier from BatchResult.
            if let Some(r) = self
                .results
                .iter()
                .find(|r| r.mode == mode && r.scenario == "S5" && r.status == ScenarioStatus::Passed)
            {
                if let Some(br) = &r.batch_result {
                    self.deltas.insert(format!("mode{}_S5_throughput_multiplier", mode), br.speedup_factor);
                }
            }
        }
    }

    pub fn to_json_pretty(&self) -> anyhow::Result<String> {
        serde_json::to_string_pretty(self).map_err(Into::into)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Environment + binary versions
// ─────────────────────────────────────────────────────────────────────────────

/// Hardware + OS disclosure - spec mandated.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EnvironmentInfo {
    pub os: String,                  // e.g. "Windows 11 Pro 23H2"
    pub cpu_model: String,           // e.g. "AMD Ryzen 9 5950X"
    pub cpu_cores: usize,
    pub total_ram_mb: u64,
    pub disk_type: String,           // best-effort: "SSD" / "HDD" / "Unknown"
    pub hostname: String,
    pub network_path: String,        // "local" if base node is on 127.0.0.1, else the URL
    pub harness_version: String,     // env!("CARGO_PKG_VERSION")
}

/// Pinned versions of the binaries the harness drove.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct BinaryVersions {
    pub console_wallet: Option<String>,  // `--version` output or commit hash
    pub minotari_cli: Option<String>,
    pub base_node: Option<String>,
    pub payment_processor: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-scenario result
// ─────────────────────────────────────────────────────────────────────────────

/// Result for one (scenario, mode) combination.
#[derive(Debug, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub scenario: String,               // "B0", "S0", ..., "S7"
    pub mode: u8,                       // 1, 2, or 3
    pub status: ScenarioStatus,
    pub error: Option<String>,

    // Timing
    pub wall_clock_secs: f64,
    pub timings: HashMap<String, f64>,  // labelled sub-timings

    // Counts / amounts
    pub utxo_count: Option<u32>,
    pub txs_sent: Option<u32>,
    pub txs_confirmed: Option<u32>,
    pub amount_transferred_ut: Option<u64>,

    // Balance reconciliation - SPEC: "expected_balance - observed_balance after every scenario"
    pub balance_before_ut: Option<u64>,
    pub balance_after_ut: Option<u64>,
    pub balance_reconciliation_delta_ut: Option<i64>,  // expected - observed; flag any non-zero

    // Fees - SPEC: per tx, per round, per scenario total
    pub total_fees_ut: Option<u64>,

    // Resource sampling (peak over scenario)
    pub peak_rss_mb: Option<u64>,
    pub peak_cpu_pct: Option<f32>,

    // Scan-specific
    pub blocks_scanned: Option<u64>,
    pub scan_from_height: Option<u64>,
    pub h_tip_start: Option<u64>,
    pub h_tip_end: Option<u64>,
    pub blocks_per_sec: Option<f64>,

    // S4
    pub concurrency_results: Option<Vec<ConcurrencyResult>>,

    // S5
    pub batch_result: Option<BatchResult>,

    // Per-tx metrics (S1, S4, S5)
    pub per_tx_metrics: Option<Vec<TxMetrics>>,

    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl ScenarioResult {
    pub fn new(scenario: impl Into<String>, mode: u8) -> Self {
        Self {
            scenario: scenario.into(),
            mode,
            status: ScenarioStatus::Running,
            error: None,
            wall_clock_secs: 0.0,
            timings: HashMap::new(),
            utxo_count: None,
            txs_sent: None,
            txs_confirmed: None,
            amount_transferred_ut: None,
            balance_before_ut: None,
            balance_after_ut: None,
            balance_reconciliation_delta_ut: None,
            total_fees_ut: None,
            peak_rss_mb: None,
            peak_cpu_pct: None,
            blocks_scanned: None,
            scan_from_height: None,
            h_tip_start: None,
            h_tip_end: None,
            blocks_per_sec: None,
            concurrency_results: None,
            batch_result: None,
            per_tx_metrics: None,
            started_at: Utc::now(),
            finished_at: None,
        }
    }

    pub fn complete(&mut self, wall_time: Duration) {
        self.wall_clock_secs = wall_time.as_secs_f64();
        self.status = ScenarioStatus::Passed;
        self.finished_at = Some(Utc::now());
    }

    pub fn fail(&mut self, wall_time: Duration, err: impl std::fmt::Display) {
        self.wall_clock_secs = wall_time.as_secs_f64();
        self.status = ScenarioStatus::Failed;
        self.error = Some(err.to_string());
        self.finished_at = Some(Utc::now());
    }

    pub fn skip(&mut self, reason: impl Into<String>) {
        self.status = ScenarioStatus::Skipped;
        self.error = Some(reason.into());
        self.finished_at = Some(Utc::now());
    }

    pub fn add_timing(&mut self, label: impl Into<String>, d: Duration) {
        self.timings.insert(label.into(), d.as_secs_f64());
    }

    /// Compute and store the balance reconciliation delta (expected - observed).
    /// Any non-zero result should be flagged in the report summary.
    #[allow(dead_code)]
    pub fn reconcile_balance(&mut self, expected_ut: i64) {
        if let Some(actual) = self.balance_after_ut {
            self.balance_reconciliation_delta_ut = Some(expected_ut - actual as i64);
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum ScenarioStatus {
    Running,
    Passed,
    Failed,
    Skipped,
}

/// Per-concurrency-level measurement for S4.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConcurrencyResult {
    pub workers: u32,
    pub txs_constructed: u32,
    pub txs_confirmed: u32,
    pub wall_secs: f64,
    pub txs_per_sec: f64,
    /// Spec: "max observed serialization gap between consecutive construction-complete events"
    pub max_serialisation_gap_secs: Option<f64>,
    /// Spec: "any double-selection rejections"
    pub double_selection_rejections: u32,
    pub success_rate: f64,
}

/// Batch vs individual comparison for S5.
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchResult {
    pub batch_size: u32,                    // K per tx
    pub batch_tx_count: u32,                // M/K
    pub batch_wall_secs: f64,
    pub batch_total_fees_ut: u64,
    pub individual_count: u32,              // M
    pub individual_wall_secs: f64,
    pub individual_total_fees_ut: u64,
    /// Headline: T_individual / T_batch
    pub speedup_factor: f64,
    pub fee_per_recipient_batch_ut: f64,
    pub fee_per_recipient_individual_ut: f64,
}

/// Per-transaction metric record (spec: construction, broadcast→mempool, broadcast→confirmed, fee, outcome).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TxMetrics {
    /// Phase label, e.g. "doubling_round_3", "fanout", "s4_n32", "s5_batch", "s5_individual"
    pub scenario_phase: String,
    pub tx_id: String,
    pub construction_secs: f64,
    pub broadcast_to_mempool_secs: f64,
    pub broadcast_to_confirmed_secs: Option<f64>,
    pub fee_paid_ut: Option<u64>,
    /// "constructed" | "confirmed" | "rejected" | "stalled" | "timed_out"
    pub outcome: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Environment capture (uses std + sysinfo where available)
// ─────────────────────────────────────────────────────────────────────────────

impl EnvironmentInfo {
    /// Capture host environment.  Best-effort: fields that can't be detected
    /// fall back to "Unknown".
    pub fn capture(network_path: String) -> Self {
        let mut sys = sysinfo::System::new_all();
        sys.refresh_all();

        let cpu_model = sys
            .cpus()
            .first()
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        let cpu_cores = sys.cpus().len();
        let total_ram_mb = sys.total_memory() / 1024 / 1024;

        let os = sysinfo::System::long_os_version().unwrap_or_else(|| "Unknown".to_string());
        let hostname = sysinfo::System::host_name().unwrap_or_else(|| "unknown".to_string());

        // Disk type detection is unreliable cross-platform; mark as Unknown unless
        // the operator overrides with TARI_BENCH_DISK_TYPE.
        let disk_type = std::env::var("TARI_BENCH_DISK_TYPE").unwrap_or_else(|_| "Unknown".to_string());

        Self {
            os,
            cpu_model,
            cpu_cores,
            total_ram_mb,
            disk_type,
            hostname,
            network_path,
            harness_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn scenario_result_new_starts_running() {
        let r = ScenarioResult::new("B0", 1);
        assert_eq!(r.scenario, "B0");
        assert_eq!(r.mode, 1);
        assert_eq!(r.status, ScenarioStatus::Running);
        assert!(r.error.is_none());
        assert!(r.finished_at.is_none());
    }

    #[test]
    fn scenario_result_complete_marks_passed() {
        let mut r = ScenarioResult::new("S0", 1);
        r.complete(Duration::from_secs(42));
        assert_eq!(r.status, ScenarioStatus::Passed);
        assert_eq!(r.wall_clock_secs, 42.0);
        assert!(r.finished_at.is_some());
    }

    #[test]
    fn scenario_result_fail_records_error_and_wall_time() {
        let mut r = ScenarioResult::new("S1", 1);
        r.fail(Duration::from_secs(5), "boom");
        assert_eq!(r.status, ScenarioStatus::Failed);
        assert_eq!(r.error.as_deref(), Some("boom"));
        assert_eq!(r.wall_clock_secs, 5.0);
    }

    #[test]
    fn scenario_result_skip_keeps_zero_wall_clock() {
        let mut r = ScenarioResult::new("S4", 1);
        r.skip("blocked by upstream");
        assert_eq!(r.status, ScenarioStatus::Skipped);
        assert_eq!(r.wall_clock_secs, 0.0);
        assert_eq!(r.error.as_deref(), Some("blocked by upstream"));
    }

    #[test]
    fn add_timing_stores_labelled_seconds() {
        let mut r = ScenarioResult::new("B0", 1);
        r.add_timing("scan", Duration::from_millis(8178));
        let scan = r.timings.get("scan").copied().expect("scan label stored");
        assert!((scan - 8.178).abs() < 1e-6);
    }

    #[test]
    fn reconcile_balance_computes_delta() {
        let mut r = ScenarioResult::new("S0", 1);
        r.balance_after_ut = Some(30_000_000_000);
        r.reconcile_balance(30_000_000_000);
        assert_eq!(r.balance_reconciliation_delta_ut, Some(0));

        r.reconcile_balance(30_000_005_000);
        assert_eq!(r.balance_reconciliation_delta_ut, Some(5_000));
    }

    #[test]
    fn benchmark_report_deltas_S2_minus_B0() {
        let mut report = BenchmarkReport::new(
            "run-id".to_string(),
            "esmeralda".to_string(),
            "sha".to_string(),
            EnvironmentInfo::capture("local".to_string()),
            BinaryVersions::default(),
            serde_json::json!({}),
        );

        let mut b0 = ScenarioResult::new("B0", 1);
        b0.add_timing("scan", Duration::from_secs(8));
        b0.complete(Duration::from_secs(8));
        report.push(b0);

        let mut s2 = ScenarioResult::new("S2", 1);
        s2.add_timing("scan", Duration::from_secs(27));
        s2.complete(Duration::from_secs(27));
        report.push(s2);

        report.finish();

        let delta = report
            .deltas
            .get("mode1_S2_minus_B0_secs")
            .copied()
            .expect("S2-B0 delta computed");
        assert!((delta - 19.0).abs() < 1e-6, "expected ~19s delta, got {}", delta);
    }

    #[test]
    fn benchmark_report_S5_throughput_multiplier_pulls_from_batch_result() {
        let mut report = BenchmarkReport::new(
            "run".into(),
            "esmeralda".into(),
            "sha".into(),
            EnvironmentInfo::capture("local".into()),
            BinaryVersions::default(),
            serde_json::json!({}),
        );

        let mut s5 = ScenarioResult::new("S5", 1);
        s5.batch_result = Some(BatchResult {
            batch_size: 10,
            batch_tx_count: 10,
            batch_wall_secs: 8.67,
            batch_total_fees_ut: 7400,
            individual_count: 100,
            individual_wall_secs: 9.30,
            individual_total_fees_ut: 78775,
            speedup_factor: 1.07,
            fee_per_recipient_batch_ut: 74.0,
            fee_per_recipient_individual_ut: 787.75,
        });
        s5.complete(Duration::from_secs(10));
        report.push(s5);
        report.finish();

        let mul = report
            .deltas
            .get("mode1_S5_throughput_multiplier")
            .copied()
            .expect("S5 multiplier in deltas");
        assert!((mul - 1.07).abs() < 1e-6);
    }

    #[test]
    fn scenario_status_serialises_lowercase() {
        let json = serde_json::to_string(&ScenarioStatus::Passed).unwrap();
        assert_eq!(json, "\"passed\"");
        let json = serde_json::to_string(&ScenarioStatus::Failed).unwrap();
        assert_eq!(json, "\"failed\"");
    }

    #[test]
    fn scenario_result_roundtrips_via_json() {
        let mut r = ScenarioResult::new("S5", 1);
        r.add_timing("batch_send", Duration::from_secs(8));
        r.complete(Duration::from_secs(10));
        let json = serde_json::to_string(&r).unwrap();
        let back: ScenarioResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.scenario, "S5");
        assert_eq!(back.status, ScenarioStatus::Passed);
    }

    #[test]
    fn environment_info_captures_nonempty_fields() {
        let env = EnvironmentInfo::capture("local".into());
        assert!(!env.os.is_empty(), "os should not be empty");
        assert!(env.cpu_cores > 0, "should detect at least one CPU core");
        assert!(env.total_ram_mb > 0, "should detect some RAM");
        assert_eq!(env.network_path, "local");
        assert!(!env.harness_version.is_empty());
    }
}
