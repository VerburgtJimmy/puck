//! Best-effort update notifications (TTY, ≤1/24h, 500ms).

use crate::fetch::{fetch_manifest, http_client};
use crate::replace::default_cache_dir;
use crate::target::detect_target;
use crate::urls::MANIFEST_URL;
use puck_version::{Operator, version_compare};
use serde::{Deserialize, Serialize};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CHECK_TIMEOUT: Duration = Duration::from_millis(500);
const MIN_INTERVAL_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CacheFile {
    /// Unix seconds of last check attempt.
    checked_at: u64,
    /// Last seen latest version from manifest (if any).
    latest_version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NotifyOptions {
    pub current_version: String,
    /// Override manifest URL (tests).
    pub manifest_url: Option<String>,
    /// Override cache path (tests).
    pub cache_path: Option<PathBuf>,
    /// Force treating stderr as TTY (tests).
    pub force_tty: Option<bool>,
    /// Skip env CI / PUCK_NO_UPDATE_CHECK checks (tests).
    pub ignore_env_disable: bool,
}

/// Best-effort: print one line to stderr if a newer version exists.
/// Never returns an error to callers that care; this function swallows failures.
pub fn maybe_notify_update(opts: NotifyOptions) {
    let _ = maybe_notify_update_inner(opts);
}

fn maybe_notify_update_inner(opts: NotifyOptions) -> Result<(), ()> {
    if !opts.ignore_env_disable {
        if std::env::var_os("CI").is_some() {
            return Ok(());
        }
        if std::env::var_os("PUCK_NO_UPDATE_CHECK").is_some() {
            return Ok(());
        }
    }

    let is_tty = opts
        .force_tty
        .unwrap_or_else(|| std::io::stderr().is_terminal());
    if !is_tty {
        return Ok(());
    }

    let cache_path = opts
        .cache_path
        .unwrap_or_else(|| default_cache_dir().join("latest"));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut cache = read_cache(&cache_path).unwrap_or_default();
    if cache.checked_at > 0 && now.saturating_sub(cache.checked_at) < MIN_INTERVAL_SECS {
        return Ok(());
    }

    // Record attempt time even on failure so we don't hammer the network.
    cache.checked_at = now;
    let _ = write_cache(&cache_path, &cache);

    let target = detect_target().map_err(|_| ())?;
    let url = opts.manifest_url.as_deref().unwrap_or(MANIFEST_URL);
    let client = http_client(CHECK_TIMEOUT, &opts.current_version, target).map_err(|_| ())?;
    let manifest = fetch_manifest(&client, url).map_err(|_| ())?;

    cache.latest_version = Some(manifest.version.clone());
    let _ = write_cache(&cache_path, &cache);

    let current = strip_v(&opts.current_version);
    let latest = strip_v(&manifest.version);
    if version_compare(latest, current, Operator::Gt) {
        eprintln!("puck: update available: {current} -> {latest} (run `puck upgrade`)");
    }
    Ok(())
}

fn strip_v(v: &str) -> &str {
    v.strip_prefix('v').unwrap_or(v)
}

fn read_cache(path: &PathBuf) -> Option<CacheFile> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_cache(path: &PathBuf, cache: &CacheFile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(cache).unwrap_or_else(|_| "{\"checked_at\":0}".into());
    std::fs::write(path, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_v_works() {
        assert_eq!(strip_v("v0.1.0"), "0.1.0");
        assert_eq!(strip_v("0.1.0"), "0.1.0");
    }
}
