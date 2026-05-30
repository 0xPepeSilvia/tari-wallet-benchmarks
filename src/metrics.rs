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

/// Complete output of a harness run. Written to `{work_dir}/results/run-{run_id}.json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub run_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub network: String,
    pub config_sha256: String,    // SHA-256 of the benchmark.toml used
    pub results: Vec<ScenarioResult>,
}

impl BenchmarkReport {
    pub fn new(run_id: String, network: String, config_sha256: String) -> Self {
        Self {
            run_id,
            started_at: Utc::now(),
            finished_at: None,
            network,
            config_sha256,
            results: Vec::new(),
        }
    }

    pub fn push(&mut self, r: ScenarioResult) {
        self.results.push(r);
    }

    pub fn finish(&mut self) {
        self.finished_at = Some(Utc::now());
    }

    pub fn to_json_pretty(&self) -> anyhow::Result<String> {
        serde_json::to_string_pretty(self).map_err(Into::into)
    }
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
    pub error: Option<String>,          // set on failure/skip

    // Timing
    pub wall_clock_secs: f64,           // total wall time from start to end of scenario
    pub timings: HashMap<String, f64>,  // labelled sub-timings (e.g. "scan", "confirm", "construct")

    // Counts / amounts produced during this scenario
    pub utxo_count: Option<u32>,
    pub txs_sent: Option<u32>,
    pub txs_confirmed: Option<u32>,
    pub amount_transferred_ut: Option<u64>,  // µT

    // Scan-specific
    pub blocks_scanned: Option<u64>,
    pub scan_from_height: Option<u64>,

    // Concurrency-specific (S4)
    pub concurrency_results: Option<Vec<ConcurrencyResult>>,

    // Batch vs individual (S5)
    pub batch_result: Option<BatchResult>,

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
            blocks_scanned: None,
            scan_from_height: None,
            concurrency_results: None,
            batch_result: None,
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
    pub wall_secs: f64,
    pub txs_per_sec: f64,
}

/// Batch vs individual comparison for S5.
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchResult {
    pub batch_size: u32,
    pub batch_wall_secs: f64,
    pub individual_count: u32,
    pub individual_wall_secs: f64,
    pub speedup_factor: f64,
}
