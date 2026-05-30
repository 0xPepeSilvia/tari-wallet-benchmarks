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
use wallet_grpc::WalletGrpcClient;

/// Smoke test: connect to a running wallet, exercise each RPC the harness uses,
/// and report success/failure per method.  Validates our proto definitions
/// against the real running wallet before we burn time on full scenarios.
async fn smoke_test(addr: &str) -> Result<()> {
    let url = if addr.starts_with("http") {
        addr.to_string()
    } else {
        format!("http://{}", addr)
    };
    println!("Smoke-testing wallet gRPC at {}", url);

    let client = WalletGrpcClient::connect(&url).await?;
    println!("  [ok] connected");

    match client.get_state().await {
        Ok(s) => println!(
            "  [ok] GetState: scanned_height={} initial_validation_done={} network_status={:?}",
            s.scanned_height,
            s.has_done_initial_validation,
            s.network.as_ref().map(|n| n.status)
        ),
        Err(e) => println!("  [FAIL] GetState: {}", e),
    }

    match client.get_balance().await {
        Ok(b) => println!(
            "  [ok] GetBalance: available={} pending_in={} pending_out={} timelocked={}",
            b.available_balance, b.pending_incoming_balance,
            b.pending_outgoing_balance, b.timelocked_balance
        ),
        Err(e) => println!("  [FAIL] GetBalance: {}", e),
    }

    match client.get_address().await {
        Ok(a) => {
            let preview: String = a.address.iter().take(8).map(|b| format!("{:02x}", b)).collect();
            println!("  [ok] GetAddress: {} bytes (prefix {}...)", a.address.len(), preview);
        },
        Err(e) => println!("  [FAIL] GetAddress: {}", e),
    }

    println!("Smoke test complete.");
    Ok(())
}

/// Rescan benchmark: trigger RescanWallet(0) on a running wallet and time it.
async fn rescan_bench(addr: &str) -> Result<()> {
    use std::time::Instant;
    let url = if addr.starts_with("http") { addr.to_string() } else { format!("http://{}", addr) };

    println!("Rescan benchmark against {}", url);
    let client = WalletGrpcClient::connect(&url).await?;

    let state_start = client.get_state().await?;
    let balance_start = client.get_balance().await?;
    let tip_target = state_start.scanned_height;

    println!("  starting scanned_height: {}", tip_target);
    println!("  balance: {} µT ({:.2} tXTM)", balance_start.available_balance, balance_start.available_balance as f64 / 1e6);
    println!("  triggering RescanWallet(from_height=0)...");

    let start = Instant::now();
    client.rescan_wallet(0).await?;

    // Poll until scanned_height returns to >= tip_target.
    let mut last_h = 0u64;
    let mut last_log = Instant::now();
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let s = client.get_state().await?;
        if s.scanned_height >= tip_target {
            let elapsed = start.elapsed();
            let bps = tip_target as f64 / elapsed.as_secs_f64();
            let balance_end = client.get_balance().await?;
            println!("\n  RESCAN COMPLETE");
            println!("  blocks_scanned: {}", tip_target);
            println!("  duration:       {:.2?}", elapsed);
            println!("  blocks/sec:     {:.2}", bps);
            println!("  balance_after:  {} µT ({:.2} tXTM)", balance_end.available_balance, balance_end.available_balance as f64 / 1e6);
            println!("  balance_delta:  {} µT", balance_end.available_balance as i64 - balance_start.available_balance as i64);

            let result = serde_json::json!({
                "scenario": "rescan_bench_S2_like",
                "mode": 1,
                "wallet_endpoint": url,
                "blocks_scanned": tip_target,
                "duration_secs": elapsed.as_secs_f64(),
                "blocks_per_sec": bps,
                "balance_start_ut": balance_start.available_balance,
                "balance_end_ut": balance_end.available_balance,
                "balance_delta_ut": balance_end.available_balance as i64 - balance_start.available_balance as i64,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            let path = "rescan_bench_result.json";
            std::fs::write(path, serde_json::to_string_pretty(&result)?)?;
            println!("\n  Result written to {}", path);
            return Ok(());
        }

        if last_log.elapsed().as_secs() >= 10 {
            let elapsed = start.elapsed();
            let bps = if elapsed.as_secs() > 0 { s.scanned_height as f64 / elapsed.as_secs_f64() } else { 0.0 };
            let pct = s.scanned_height as f64 / tip_target as f64 * 100.0;
            println!("  scanned_height: {}/{} ({:.1}%) - {:.1} blocks/s - elapsed {:.0}s",
                s.scanned_height, tip_target, pct, bps, elapsed.as_secs_f64());
            last_log = Instant::now();
        }
        last_h = s.scanned_height;
        let _ = last_h;
    }
}

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

    /// Smoke-test the gRPC connection to a running wallet (host:port form).
    /// Validates that our proto definitions match the real wallet service.
    /// Skips all scenarios.
    #[arg(long)]
    smoke: Option<String>,

    /// Benchmark a rescan-from-0 against a running wallet at (host:port).
    /// Triggers RescanWallet(from_height=0), polls GetState until scanned_height
    /// reaches the chain tip captured at start, writes a JSON result.
    /// Equivalent to S2 against an existing wallet (skips the fresh-seed wipe).
    #[arg(long)]
    rescan_bench: Option<String>,

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

    if let Some(addr) = args.smoke.as_ref() {
        return smoke_test(addr).await;
    }

    if let Some(addr) = args.rescan_bench.as_ref() {
        return rescan_bench(addr).await;
    }

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
