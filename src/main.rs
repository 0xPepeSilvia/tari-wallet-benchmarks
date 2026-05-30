// Tari Wallet Benchmark Harness
//
// Reproducible performance test harness for the Tari wallet stack.
// Implements scenarios B0 and S0-S7 across three wallet modes.
//
// Usage:
//   tari-wallet-benchmarks --config benchmark.toml
//   tari-wallet-benchmarks --config benchmark.toml --modes 1,2 --scenarios B0,S0,S1
//   tari-wallet-benchmarks --config benchmark.toml --list-scenarios

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::{fmt, EnvFilter};

mod config;
mod helpers;
mod metrics;
mod modes;
mod runner;
mod scenarios;
mod wallet_grpc;

use config::BenchmarkConfig;
use runner::Runner;

// ─────────────────────────────────────────────────────────────────────────────
// CLI
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "tari-wallet-benchmarks",
    about = "Reproducible Tari wallet performance test harness (B0/S0-S7, Modes 1-3)",
    version
)]
struct Args {
    /// Path to the benchmark configuration file.
    #[arg(short, long, default_value = "benchmark.toml")]
    config: PathBuf,

    /// Override: comma-separated modes to run (e.g. "1,2").
    /// Overrides config.modes.
    #[arg(long, value_delimiter = ',')]
    modes: Option<Vec<u8>>,

    /// Override: comma-separated scenarios to run (e.g. "B0,S0,S1").
    /// Overrides config.scenarios.
    #[arg(long, value_delimiter = ',')]
    scenarios: Option<Vec<String>>,

    /// List all available scenarios and exit.
    #[arg(long)]
    list_scenarios: bool,

    /// Output directory for result JSON files.
    /// Overrides config.work_dir if provided.
    #[arg(long)]
    output_dir: Option<PathBuf>,

    /// Log level (trace, debug, info, warn, error).
    #[arg(long, env = "TARI_BENCH_LOG", default_value = "info")]
    log_level: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialise structured logging.
    let filter = EnvFilter::try_new(&args.log_level)
        .unwrap_or_else(|_| EnvFilter::new("info"));
    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(false)
        .init();

    if args.list_scenarios {
        println!("Available scenarios:");
        for s in &["B0", "S0", "S1", "S2", "S3", "S4", "S5", "S6", "S7"] {
            println!("  {}", s);
        }
        println!();
        println!("Available modes: 1 (ConsoleWallet), 2 (MinotariCli), 3 (PaymentProcessor)");
        return Ok(());
    }

    let mut cfg = BenchmarkConfig::load(&args.config)?;

    // Apply CLI overrides.
    if let Some(modes) = args.modes {
        cfg.modes = modes;
    }
    if let Some(scenarios) = args.scenarios {
        cfg.scenarios = scenarios;
    }
    if let Some(output_dir) = args.output_dir {
        cfg.work_dir = output_dir;
    }

    // Ensure work_dir exists.
    std::fs::create_dir_all(&cfg.work_dir)?;

    let runner = Runner::new(cfg, args.config.clone())?;
    let report = runner.run().await?;

    // Exit non-zero if any scenario failed.
    let failures = report.results.iter()
        .filter(|r| r.status == metrics::ScenarioStatus::Failed)
        .count();

    if failures > 0 {
        eprintln!("{} scenario(s) failed.", failures);
        std::process::exit(1);
    }

    Ok(())
}
