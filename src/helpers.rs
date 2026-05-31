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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_seed_entropy_returns_32_bytes() {
        let e = fresh_seed_entropy();
        assert_eq!(e.len(), 32);
    }

    #[test]
    fn fresh_seed_entropy_is_nonzero() {
        let e = fresh_seed_entropy();
        let zeroes = [0u8; 32];
        assert_ne!(e, zeroes, "entropy should not be all zeroes");
    }

    #[test]
    fn fresh_seed_entropy_unique_across_calls() {
        let a = fresh_seed_entropy();
        let b = fresh_seed_entropy();
        assert_ne!(a, b, "two consecutive fresh seeds should differ");
    }

    #[test]
    fn entropy_to_hex_is_64_chars() {
        let e = [0u8; 32];
        let h = entropy_to_hex(&e);
        assert_eq!(h.len(), 64);
        assert_eq!(h, "0".repeat(64));
    }

    #[test]
    fn entropy_to_hex_round_trips() {
        let e = fresh_seed_entropy();
        let h = entropy_to_hex(&e);
        let back: Vec<u8> = hex::decode(&h).expect("hex encodes valid output");
        assert_eq!(back, e.to_vec());
    }

    #[test]
    fn sha256_file_known_content() {
        // SHA-256 of "hello\n" is known: 5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, b"hello\n").unwrap();
        let h = sha256_file(&path).expect("hash known file");
        assert_eq!(h, "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03");
    }

    #[test]
    fn sha256_file_empty_returns_known_constant() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        std::fs::write(&path, b"").unwrap();
        let h = sha256_file(&path).unwrap();
        // SHA-256 of empty bytes
        assert_eq!(h, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn sha256_file_errors_on_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.txt");
        assert!(sha256_file(&missing).is_err());
    }

    #[test]
    fn wipe_data_dir_removes_files_and_recreates() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("walletdir");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("wallet.db"), b"data").unwrap();
        std::fs::write(target.join("wallet.db-wal"), b"data").unwrap();
        assert!(target.join("wallet.db").exists());

        wipe_data_dir(&target).unwrap();
        assert!(target.exists(), "wipe_data_dir should leave parent dir present");
        assert!(!target.join("wallet.db").exists(), "wallet.db should be gone");
    }

    #[test]
    fn wipe_data_dir_creates_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("nonexistent");
        assert!(!target.exists());
        wipe_data_dir(&target).unwrap();
        assert!(target.exists(), "wipe_data_dir should create the dir if missing");
    }
}
