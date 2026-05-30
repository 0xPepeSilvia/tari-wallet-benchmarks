// Shared helpers: binary version capture, data dir lifecycle, fresh seed
// generation, SHA-256 hashing for config reproducibility.

use crate::config::BenchmarkConfig;
use crate::metrics::BinaryVersions;
use anyhow::Result;
use rand::RngCore;
use std::path::Path;
use std::process::Command;
use tracing::warn;

/// Capture `--version` output (or commit hash) for each binary referenced in config.
/// SPEC: "Pinned versions: minotari_console_wallet, minotari-cli, base node - commit hash or release tag."
pub fn capture_binary_versions(config: &BenchmarkConfig) -> BinaryVersions {
    BinaryVersions {
        console_wallet: try_version(&config.binaries.console_wallet),
        minotari_cli: try_version(&config.binaries.minotari_cli),
        base_node: try_version(&config.binaries.base_node),
        payment_processor: config
            .binaries
            .payment_processor
            .as_ref()
            .and_then(|p| try_version(p.as_path())),
    }
}

fn try_version(path: &Path) -> Option<String> {
    Command::new(path)
        .arg("--version")
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                None
            }
        })
}

/// SHA-256 of the config file bytes.  Recorded in the result profile so a
/// reviewer can confirm two runs used the same config.
pub fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

/// Wipe a wallet data directory.  Used before B0, S2, S3, S6, S7 per spec:
/// "Every scan scenario starts with a wiped wallet data dir."
#[allow(dead_code)]
pub fn wipe_data_dir(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)
            .map_err(|e| anyhow::anyhow!("Failed to wipe {:?}: {}", path, e))?;
    }
    std::fs::create_dir_all(path)?;
    Ok(())
}

/// Generate 32 bytes of entropy for a fresh wallet seed.
/// SPEC: "fresh seed per wallet mode."
#[allow(dead_code)]
pub fn fresh_seed_entropy() -> [u8; 32] {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    buf
}

/// Encode entropy as a hex string suitable for passing to a wallet --import-seed flag.
#[allow(dead_code)]
pub fn entropy_to_hex(entropy: &[u8; 32]) -> String {
    hex::encode(entropy)
}

/// Best-effort: log a warning rather than fail the whole run.
#[allow(dead_code)]
pub fn warn_on_err<T, E: std::fmt::Display>(label: &str, r: std::result::Result<T, E>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            warn!("{}: {}", label, e);
            None
        },
    }
}
