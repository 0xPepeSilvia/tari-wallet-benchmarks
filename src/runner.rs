// Orchestrates mode × scenario execution and accumulates results.
//
// The runner iterates over the configured modes and scenarios, builds a
// WalletMode instance for each, runs each scenario in order, and collects
// ScenarioResults into a BenchmarkReport.

use crate::config::BenchmarkConfig;
use crate::metrics::BenchmarkReport;
use crate::modes::build_mode;
use crate::scenarios::run_scenario;
use anyhow::Result;
use std::path::PathBuf;
use tracing::{error, info, warn};
use uuid::Uuid;

// Ordered scenario list from the spec.  We always run in this order even if
// only a subset is selected - scenarios have implicit ordering dependencies
// (S1 must precede S2, S2 must precede S4, etc.).
const SCENARIO_ORDER: &[&str] = &["B0", "S0", "S1", "S2", "S3", "S4", "S5", "S6", "S7"];

pub struct Runner {
    config: BenchmarkConfig,
    run_id: String,
    results_dir: PathBuf,
}

impl Runner {
    pub fn new(config: BenchmarkConfig) -> Result<Self> {
        let run_id = Uuid::new_v4().to_string();
        let results_dir = config.work_dir.join("results");
        std::fs::create_dir_all(&results_dir)?;
        Ok(Self { config, run_id, results_dir })
    }

    pub async fn run(&self) -> Result<BenchmarkReport> {
        let config_sha = sha256_config_file()?;
        let mut report = BenchmarkReport::new(
            self.run_id.clone(),
            self.config.network.clone(),
            config_sha,
        );

        info!("=== Tari Wallet Benchmark Harness ===");
        info!("Run ID:   {}", self.run_id);
        info!("Network:  {}", self.config.network);
        info!("Modes:    {:?}", self.config.modes);
        info!("Scenarios: {:?}", self.config.scenarios);

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
                // Skip scenarios not in the configured list.
                if !self.config.scenarios.iter().any(|s| s == scenario_name) {
                    continue;
                }

                let result = run_scenario(scenario_name, mode.as_mut(), &self.config).await;
                let status = result.status;
                report.push(result);

                // Persist an intermediate result file after each scenario in case
                // the run aborts partway through.
                if let Err(e) = self.write_intermediate(&report) {
                    warn!("Failed to write intermediate results: {}", e);
                }

                // If S0 failed (funding), no point running S1-S7.
                if scenario_name == "S0" && status == crate::metrics::ScenarioStatus::Failed {
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
        // Remove partial file if it exists.
        let partial = self.results_dir.join(format!("run-{}-partial.json", self.run_id));
        let _ = std::fs::remove_file(&partial);

        let path = self.results_dir.join(format!("run-{}.json", self.run_id));
        let json = report.to_json_pretty()?;
        std::fs::write(&path, &json)?;
        info!("Results written to {:?}", path);
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
        println!("{}", "─".repeat(80));

        for r in &report.results {
            let notes = if let Some(ref e) = r.error {
                e.chars().take(40).collect::<String>()
            } else if let Some(ref cr) = r.concurrency_results {
                let best = cr.iter()
                    .filter(|c| c.txs_constructed > 0)
                    .map(|c| format!("N{}={:.1}tx/s", c.workers, c.txs_per_sec))
                    .collect::<Vec<_>>()
                    .join(", ");
                best
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
        println!();
    }
}

fn sha256_config_file() -> Result<String> {
    // Placeholder - in a real run we'd hash the actual config file path.
    // This is populated by main() where the path is known.
    Ok("unknown".to_string())
}
