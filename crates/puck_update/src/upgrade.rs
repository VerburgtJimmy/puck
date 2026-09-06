//! `puck upgrade` / `--rollback` implementation.

use crate::error::{Error, Result};
use crate::fetch::{fetch_bytes, fetch_manifest, http_client};
use crate::homebrew::is_homebrew_managed;
use crate::replace::{atomic_replace, rollback as swap_previous};
use crate::target::detect_target;
use crate::urls::{MANIFEST_URL, manifest_url_for_version};
use crate::verify::{verify_minisign_optional, verify_sha256};
use flate2::read::GzDecoder;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tar::Archive;

const UPGRADE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct UpgradeOptions {
    /// Pin to this version (`0.1.0` / `v0.1.0`). `None` = latest manifest.
    pub version: Option<String>,
    /// Override manifest URL (tests).
    pub manifest_url: Option<String>,
    /// Current package version string (for User-Agent).
    pub current_version: String,
    /// Override executable path (tests). Defaults to `std::env::current_exe()`.
    pub current_exe: Option<PathBuf>,
    /// Working directory for download/extract (tests). Defaults to a temp dir under cache.
    pub work_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct UpgradeOutcome {
    pub from_version: String,
    pub to_version: String,
    pub target: String,
    pub path: PathBuf,
    pub minisign_verified: bool,
    pub minisign_skipped_warning: Option<String>,
}

/// Run self-upgrade against the release manifest.
pub fn upgrade(opts: UpgradeOptions) -> Result<UpgradeOutcome> {
    let current_exe = match opts.current_exe {
        Some(p) => p,
        None => std::env::current_exe().map_err(Error::Io)?,
    };

    if is_homebrew_managed(&current_exe) {
        return Err(Error::HomebrewManaged(current_exe));
    }

    let target = detect_target()?;
    let manifest_url = opts
        .manifest_url
        .clone()
        .unwrap_or_else(|| match &opts.version {
            Some(v) => manifest_url_for_version(v),
            None => MANIFEST_URL.to_owned(),
        });

    let client = http_client(UPGRADE_TIMEOUT, &opts.current_version, target)?;
    let manifest = fetch_manifest(&client, &manifest_url)?;
    if let Some(channel) = &manifest.channel {
        if channel != "stable" {
            return Err(Error::Manifest(format!(
                "refusing non-stable channel `{channel}` in 0.1"
            )));
        }
    }

    let artifact = manifest.artifact_for(target)?;
    let archive_bytes = fetch_bytes(&client, &artifact.url)?;
    verify_sha256(&archive_bytes, &artifact.sha256, &artifact.url)?;

    let work = opts.work_dir.clone().unwrap_or_else(|| {
        let dir = crate::replace::default_cache_dir().join("upgrade");
        let _ = std::fs::create_dir_all(&dir);
        dir
    });
    std::fs::create_dir_all(&work)?;

    let archive_name = artifact
        .url
        .rsplit('/')
        .next()
        .unwrap_or("puck-archive.tar.gz");
    let archive_path = work.join(archive_name);
    std::fs::write(&archive_path, &archive_bytes)?;

    // Optional minisign over SHA256SUMS (same policy as install.sh).
    let mut minisign_verified = false;
    let mut minisign_skipped_warning = None;
    let sums_url = artifact
        .url
        .rsplit_once('/')
        .map(|(base, _)| format!("{base}/SHA256SUMS"));
    if let Some(sums_url) = sums_url {
        let sig_url = format!("{sums_url}.minisig");
        match (
            fetch_bytes(&client, &sums_url),
            fetch_bytes(&client, &sig_url),
        ) {
            (Ok(sums), Ok(sig)) => {
                let sums_path = work.join("SHA256SUMS");
                let sig_path = work.join("SHA256SUMS.minisig");
                std::fs::write(&sums_path, &sums)?;
                std::fs::write(&sig_path, &sig)?;
                // Confirm our artifact hash is listed.
                let expected_line = format!("{}  {}", artifact.sha256, archive_name);
                let alt_line = format!("{} *{}", artifact.sha256, archive_name);
                let sums_txt = String::from_utf8_lossy(&sums);
                if !sums_txt.lines().any(|l| {
                    l.trim() == expected_line || l.trim() == alt_line || {
                        let parts: Vec<_> = l.split_whitespace().collect();
                        parts.len() >= 2
                            && parts[0].eq_ignore_ascii_case(&artifact.sha256)
                            && parts[1].trim_start_matches('*') == archive_name
                    }
                }) {
                    return Err(Error::Manifest(format!(
                        "SHA256SUMS missing entry for {archive_name}"
                    )));
                }
                match verify_minisign_optional(&sums_path, &sig_path)? {
                    true => minisign_verified = true,
                    false => {
                        minisign_skipped_warning = Some(
                            "minisign not installed; verified sha256 only".into(),
                        );
                    }
                }
            }
            (Ok(sums), Err(_)) => {
                // Sums without signature: still check listing; sha256 already verified.
                let sums_txt = String::from_utf8_lossy(&sums);
                let listed = sums_txt.lines().any(|l| {
                    let parts: Vec<_> = l.split_whitespace().collect();
                    parts.len() >= 2
                        && parts[0].eq_ignore_ascii_case(&artifact.sha256)
                        && parts[1].trim_start_matches('*') == archive_name
                });
                if !listed {
                    // Non-fatal: manifest sha256 already checked.
                }
                minisign_skipped_warning =
                    Some("SHA256SUMS.minisig missing; verified sha256 only".into());
            }
            _ => {
                minisign_skipped_warning =
                    Some("SHA256SUMS not fetched; verified sha256 from manifest only".into());
            }
        }
    }

    let extracted = extract_puck_binary(&archive_path, &work)?;
    // Belt-and-suspenders: re-hash extracted binary is not in manifest; trust archive.

    atomic_replace(&current_exe, &extracted)?;

    Ok(UpgradeOutcome {
        from_version: opts.current_version,
        to_version: manifest.version,
        target: target.to_owned(),
        path: current_exe,
        minisign_verified,
        minisign_skipped_warning,
    })
}

/// Roll back to `puck.previous` beside the current executable.
pub fn rollback_exe(current_exe: Option<PathBuf>) -> Result<PathBuf> {
    let current_exe = match current_exe {
        Some(p) => p,
        None => std::env::current_exe().map_err(Error::Io)?,
    };
    if is_homebrew_managed(&current_exe) {
        return Err(Error::HomebrewManaged(current_exe));
    }
    swap_previous(&current_exe)?;
    Ok(current_exe)
}

fn extract_puck_binary(archive_path: &Path, dest_dir: &Path) -> Result<PathBuf> {
    let file = std::fs::File::open(archive_path)?;
    let gz = GzDecoder::new(file);
    let mut archive = Archive::new(gz);
    let out = dest_dir.join("puck.extracted");
    let _ = std::fs::remove_file(&out);

    let mut found = false;
    for entry in archive
        .entries()
        .map_err(|e| Error::Archive(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| Error::Archive(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| Error::Archive(e.to_string()))?
            .to_path_buf();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if name == "puck" || name == "puck.exe" {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|e| Error::Archive(e.to_string()))?;
            std::fs::write(&out, &bytes)?;
            found = true;
            break;
        }
    }

    if !found {
        return Err(Error::Archive(
            "tarball did not contain a `puck` binary".into(),
        ));
    }
    Ok(out)
}

