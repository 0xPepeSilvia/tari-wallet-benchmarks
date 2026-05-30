// Orchestrates mode × scenario execution and accumulates results.

use crate::config::BenchmarkConfig;
use crate::helpers::{capture_binary_versions, sha256_file};
use crate::metrics::{BenchmarkReport, EnvironmentInfo, ScenarioStatus};
use crate::modes::build_mode;
use crate::scenarios::run_scenario;
use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};
use uuid::Uuid;

// Canonical scenario order per spec.  Scenarios that wipe the wallet data dir
// (B0, S2, S3, S6, S7) are tagged here so the runner can invoke
// `wipe_and_reinit` before them.
const SCENARIO_ORDER: &[&str] = &["B0", "S0", "S1", "S2", "S3", "S4", "S5", "S6", "S7"];
const SCENARIOS_REQUIRING_WIPE: &[&str] = &["B0", "S2", "S3", "S6", "S7"];

pub struct Runner {
    config: BenchmarkConfig,
    config_path: PathBuf,
    run_id: String,
    results_dir: PathBuf,
}

impl Runner {
    pub fn new(config: BenchmarkConfig, config_path: PathBuf) -> Result<Self> {
        let run_id = Uuid::new_v4().to_string();
        let results_dir = config.work_dir.join("results");
        std::fs::create_dir_all(&results_dir)?;
        Ok(Self { config, config_path, run_id, results_dir })
    }

    pub async fn run(&self) -> Result<BenchmarkReport> {
        // ── Capture run metadata ──────────────────────────────────────────────
        let config_sha = sha256_file(&self.config_path)
            .unwrap_or_else(|_| "unknown".to_string());

        let network_path = if self.config.node.http_url.contains("127.0.0.1")
            || self.config.node.http_url.contains("localhost")
        {
            "local".to_string()
        } else {
            self.config.node.http_url.clone()
        };
        let environment = EnvironmentInfo::capture(network_path);
        let binary_versions = capture_binary_versions(&self.config);
        let config_snapshot = serde_json::to_value(&self.config)?;

        let mut report = BenchmarkReport::new(
            self.run_id.clone(),
            self.config.network.clone(),
            config_sha,
            environment,
            binary_versions,
            config_snapshot,
        );

        info!("=== Tari Wallet Benchmark Harness ===");
        info!("Run ID:    {}", self.run_id);
        info!("Network:   {}", self.config.network);
        info!("Modes:     {:?}", self.config.modes);
        info!("Scenarios: {:?}", self.config.scenarios);
        info!("CPU:       {} ({} cores)", report.environment.cpu_model, report.environment.cpu_cores);
        info!("RAM:       {} MB", report.environment.total_ram_mb);
        info!("OS:        {}", report.environment.os);

        for &mode_num in &self.config.modes {
            info!("─── Mode {} ───────────────────────────────", mode_num);

            let mut mode = match build_mode(mode_num, &self.config, &self.run_id).await {
                Ok(m) => m,
                Err(e) => {
                    error!("Failed to build mode {}: {}", mode_num, e);
                    continue;
                },
            };

            if let Err(e) = mode.start().await {
                error!("Mode {} failed to start: {}", mode_num, e);
                continue;
            }

            for &scenario_name in SCENARIO_ORDER {
                if !self.config.scenarios.iter().any(|s| s == scenario_name) {
                    continue;
                }

                // SPEC: wipe wallet data dir before B0, S2, S3, S6, S7.
                if SCENARIOS_REQUIRING_WIPE.contains(&scenario_name) {
                    info!("[{}/{}] wiping wallet data dir before scan scenario", scenario_name, mode.name());
                    if let Err(e) = mode.wipe_and_reinit().await {
                        warn!("wipe_and_reinit failed for {}/{}: {}", scenario_name, mode.name(), e);
                    }
                    // For B0 birthday must be 0 (genesis).
                    // For S2/S6 birthday = 0; for S3/S7 birthday = H_birth.
                    let birthday = match scenario_name {
                        "B0" | "S2" | "S6" => 0u64,
                        "S3" | "S7" => parse_env_birthday().unwrap_or(0),
                        _ => 0,
                    };
                    let _ = mode.set_birthday(birthday).await;
                }

                let result = run_scenario(scenario_name, mode.as_mut(), &self.config).await;
                let status = result.status;
                report.push(result);

                if let Err(e) = self.write_intermediate(&report) {
                    warn!("Failed to write intermediate results: {}", e);
                }

                if scenario_name == "S0" && status == ScenarioStatus::Failed {
                    error!("S0 (funding) failed for mode {} - skipping remaining scenarios for this mode", mode_num);
                    break;
                }
            }

            if let Err(e) = mode.stop().await {
                warn!("Mode {} stop error: {}", mode_num, e);
            }
        }

        report.finish();
        self.write_final(&report)?;
        self.print_summary(&report);
        Ok(report)
    }

    fn write_intermediate(&self, report: &BenchmarkReport) -> Result<()> {
        let path = self.results_dir.join(format!("run-{}-partial.json", self.run_id));
        let json = report.to_json_pretty()?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    fn write_final(&self, report: &BenchmarkReport) -> Result<()> {
        let partial = self.results_dir.join(format!("run-{}-partial.json", self.run_id));
        let _ = std::fs::remove_file(&partial);

        let path = self.results_dir.join(format!("run-{}.json", self.run_id));
        let json = report.to_json_pretty()?;
        std::fs::write(&path, &json)?;
        info!("Results written to {:?}", path);

        // Also update the canonical baseline pointer.
        let baseline = self.results_dir.join("latest.json");
        let _ = std::fs::copy(&path, &baseline);

        Ok(())
    }

    fn print_summary(&self, report: &BenchmarkReport) {
        println!("\n=== BENCHMARK SUMMARY ===");
        println!("Run:     {}", report.run_id);
        println!("Network: {}", report.network);
        println!();

        let max_scenario = report.results.iter()
            .map(|r| r.scenario.len())
            .max()
            .unwrap_or(8);

        println!(
            "{:<width$}  Mode  Status   Wall(s)   Blocks  TxSent  Notes",
            "Scenario", width = max_scenario
        );
        println!("{}", "-".repeat(80));

        for r in &report.results {
            let notes = if let Some(ref e) = r.error {
                e.chars().take(40).collect::<String>()
            } else if let Some(ref cr) = r.concurrency_results {
                cr.iter()
                    .filter(|c| c.txs_constructed > 0)
                    .map(|c| format!("N{}={:.1}tx/s", c.workers, c.txs_per_sec))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                String::new()
            };

            println!(
                "{:<width$}  {:>4}  {:<8} {:>8.2}  {:>6?}  {:>6?}  {}",
                r.scenario,
                r.mode,
                format!("{:?}", r.status),
                r.wall_clock_secs,
                r.blocks_scanned,
                r.txs_sent,
                notes,
                width = max_scenario,
            );
        }

        if !report.deltas.is_empty() {
            println!("\nDeltas:");
            for (k, v) in &report.deltas {
                println!("  {} = {:.3}", k, v);
            }
        }
        println!();
    }
}

fn parse_env_birthday() -> Option<u64> {
    std::env::var("TARI_BENCH_BIRTHDAY_HEIGHT")
        .ok()
        .and_then(|s| s.parse().ok())
}

// Suppress unused warning - retained for future API stability.
#[allow(dead_code)]
fn _ensure_path_is_owned(p: &Path) {
    let _ = p;
}
