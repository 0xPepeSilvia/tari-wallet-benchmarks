// Scenario implementations - B0 and S0-S7.
//
// Each scenario function takes a mutable WalletMode reference and a config
// reference.  It returns a ScenarioResult with all measurements populated.

pub mod b0;
pub mod s0;
pub mod s1;
pub mod s2;
pub mod s3;
pub mod s4;
pub mod s5;
pub mod s6;
pub mod s7;

use crate::config::BenchmarkConfig;
use crate::metrics::ScenarioResult;
use crate::modes::WalletMode;
use anyhow::Result;
use tracing::info;

/// Run a single named scenario against the given wallet mode.
/// Returns a completed (passed, failed or skipped) ScenarioResult.
pub async fn run_scenario(
    name: &str,
    mode: &mut dyn WalletMode,
    config: &BenchmarkConfig,
) -> ScenarioResult {
    info!("=== Running {} on {} ===", name, mode.name());
    let mut result = ScenarioResult::new(name, mode.mode_number());

    let started = std::time::Instant::now();
    let outcome: Result<()> = match name {
        "B0" => b0::run(mode, config, &mut result).await,
        "S0" => s0::run(mode, config, &mut result).await,
        "S1" => s1::run(mode, config, &mut result).await,
        "S2" => s2::run(mode, config, &mut result).await,
        "S3" => s3::run(mode, config, &mut result).await,
        "S4" => s4::run(mode, config, &mut result).await,
        "S5" => s5::run(mode, config, &mut result).await,
        "S6" => s6::run(mode, config, &mut result).await,
        "S7" => s7::run(mode, config, &mut result).await,
        other => {
            result.skip(format!("Unknown scenario '{}'", other));
            return result;
        },
    };

    let elapsed = started.elapsed();
    match outcome {
        Ok(()) => {
            result.complete(elapsed);
            info!("{}/{} PASSED in {:.2?}", name, mode.name(), elapsed);
        },
        Err(e) => {
            result.fail(elapsed, &e);
            tracing::error!("{}/{} FAILED after {:.2?}: {}", name, mode.name(), elapsed, e);
        },
    }
    result
}
