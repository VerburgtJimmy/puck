//! SHA-256 and optional minisign verification.

use crate::error::{Error, Result};
use crate::urls::MINISIGN_PUBLIC_KEY;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

pub fn verify_sha256(bytes: &[u8], expected: &str, name: &str) -> Result<()> {
    let actual = sha256_hex(bytes);
    if actual.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        Err(Error::Sha256Mismatch {
            name: name.to_owned(),
            expected: expected.trim().to_owned(),
            actual,
        })
    }
}

/// Verify `SHA256SUMS` with minisign when the `minisign` binary is on PATH.
/// Returns `Ok(true)` if verified, `Ok(false)` if minisign was skipped (absent),
/// or `Err` on verification failure.
pub fn verify_minisign_optional(sums_path: &Path, sig_path: &Path) -> Result<bool> {
    if !command_exists("minisign") {
        return Ok(false);
    }
    if !sig_path.is_file() {
        return Err(Error::Minisign(
            "SHA256SUMS.minisig missing; cannot verify with minisign".into(),
        ));
    }

    let dir = tempfile_dir()?;
    let pub_path = dir.join("minisign.pub");
    std::fs::write(&pub_path, MINISIGN_PUBLIC_KEY)?;

    let status = Command::new("minisign")
        .args([
            "-Vm",
            &sums_path.to_string_lossy(),
            "-x",
            &sig_path.to_string_lossy(),
            "-p",
            &pub_path.to_string_lossy(),
        ])
        .status()
        .map_err(|e| Error::Minisign(format!("failed to run minisign: {e}")))?;

    // Best-effort cleanup
    let _ = std::fs::remove_dir_all(&dir);

    if status.success() {
        Ok(true)
    } else {
        Err(Error::Minisign("signature check failed".into()))
    }
}

fn command_exists(name: &str) -> bool {
    which_path(name)
}

fn which_path(name: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return true;
        }
    }
    false
}

fn tempfile_dir() -> Result<std::path::PathBuf> {
    let mut path = std::env::temp_dir();
    let unique = format!(
        "puck-minisign-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    path.push(unique);
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_empty() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
