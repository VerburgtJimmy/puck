//! Execute an install plan: fetch, store, link, write installed metadata.

use crate::installed_php::{RootPackageMeta, build_installed_php};
use crate::plan::{InstallAction, InstallOptions, InstallPlan, PlannedPackage};
use crate::{Error, Result};
use puck_dist::{ArchiveKind, AuthStore, download_with_client, http_client};
use puck_lock::{LockFile, LockedPackage};
use puck_manifest::Manifest;
use puck_store::{Store, link_tree, lookup, put_archive, remember};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// Composer default `max-parallel-http`.
const DEFAULT_HTTP_PARALLEL: usize = 12;

/// Download concurrency: `options.http_parallel`, else `config.max-parallel-http`,
/// else `PUCK_MAX_PARALLEL`, else 12.
pub fn resolve_http_parallel(options: InstallOptions, manifest: Option<&Manifest>) -> usize {
    if let Some(n) = options.http_parallel {
        return n.max(1);
    }
    if let Some(m) = manifest
        && let Some(n) = config_max_parallel_http(m)
    {
        return n.max(1);
    }
    if let Ok(raw) = std::env::var("PUCK_MAX_PARALLEL")
        && let Ok(n) = raw.trim().parse::<usize>()
    {
        return n.max(1);
    }
    DEFAULT_HTTP_PARALLEL
}

fn config_max_parallel_http(manifest: &Manifest) -> Option<usize> {
    let v = manifest.config.get("max-parallel-http")?;
    match v {
        Value::Number(n) => n.as_u64().map(|u| u as usize),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Extract worker pool size: `min(CPUs, 8)`.
fn extract_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(DEFAULT_HTTP_PARALLEL)
        .min(8)
        .max(1)
}

/// Link concurrency: hardlinks are metadata-heavy; prefer at least 8, up to CPUs.
fn link_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(DEFAULT_HTTP_PARALLEL)
        .max(8)
}

/// Wall-clock phase timings from [`execute_install`] (milliseconds).
#[derive(Debug, Clone, Default)]
pub struct ExecuteTimings {
    /// Store lookup / download / extract / link pipeline (parallel wall clock).
    pub fetch_ms: u128,
    /// Sum of per-package cache-hit lookup times (may exceed wall under concurrency).
    pub fetch_cache_hit_ms: u128,
    /// Sum of per-package download+extract times (legacy; prefer [`Self::download_ms`] /
    /// [`Self::extract_ms`]).
    pub fetch_download_ms: u128,
    /// Sum of per-package HTTP download durations.
    pub download_ms: u128,
    /// Sum of per-package extract durations.
    pub extract_ms: u128,
    /// `download_ms + extract_ms - download_extract_wall` (pipelining benefit).
    pub overlap_ms: u128,
    /// Vendor hardlink/copy wall clock (first link start → last link end).
    pub link_ms: u128,
    /// `installed.json` / `installed.php` + bins.
    pub installed_meta_ms: u128,
}

/// Run the plan against `project_root`.
pub async fn execute_install(
    project_root: &Path,
    lock: &LockFile,
    plan: &InstallPlan,
    options: InstallOptions,
    store: &Store,
    manifest: Option<&Manifest>,
) -> Result<ExecuteTimings> {
    let vendor = project_root.join("vendor");
    fs::create_dir_all(&vendor).map_err(|source| Error::Io {
        path: vendor.display().to_string(),
        source,
    })?;

    // Removals first.
    for pkg in plan.to_remove() {
        remove_package(&vendor, &pkg.name)?;
    }

    let lock_by_name: HashMap<String, &LockedPackage> = lock
        .packages
        .iter()
        .chain(lock.packages_dev.iter())
        .map(|p| (p.name.to_ascii_lowercase(), p))
        .collect();

    let to_install: Vec<&PlannedPackage> = plan.to_install().collect();
    let (path_pkgs, archive_pkgs): (Vec<&PlannedPackage>, Vec<&PlannedPackage>) =
        to_install.iter().copied().partition(|p| is_path_dist(p));

    // Path dist: symlink or mirror from project-relative url (no store fetch).
    for pkg in &path_pkgs {
        let Some(locked) = lock_by_name.get(&pkg.name) else {
            return Err(Error::Message(format!(
                "path package {} missing from lock",
                pkg.name
            )));
        };
        install_path_package(project_root, &vendor, pkg, locked)?;
        eprintln!("puck: {} {}", action_word(pkg.action), pkg.name);
    }

    let http_parallel = resolve_http_parallel(options, manifest);
    let pipeline_started = Instant::now();
    let archive_timings =
        install_archives(&archive_pkgs, store, &vendor, options.offline, http_parallel).await?;
    let fetch_ms = pipeline_started.elapsed().as_millis();

    let meta_started = Instant::now();
    write_installed_json(&vendor, lock, plan, &lock_by_name, manifest, options)?;
    crate::bins::install_binaries(project_root, lock, options.no_dev)?;
    let installed_meta_ms = meta_started.elapsed().as_millis();

    Ok(ExecuteTimings {
        fetch_ms,
        fetch_cache_hit_ms: archive_timings.cache_hit_ms,
        fetch_download_ms: archive_timings.download_ms + archive_timings.extract_ms,
        download_ms: archive_timings.download_ms,
        extract_ms: archive_timings.extract_ms,
        overlap_ms: archive_timings.overlap_ms,
        link_ms: archive_timings.link_ms,
        installed_meta_ms,
    })
}

struct ArchiveTimings {
    cache_hit_ms: u128,
    download_ms: u128,
    extract_ms: u128,
    overlap_ms: u128,
    link_ms: u128,
}

/// Per-package pipeline: lookup → (download ‖ extract pool) → link-as-you-go.
async fn install_archives(
    packages: &[&PlannedPackage],
    store: &Store,
    vendor: &Path,
    offline: bool,
    http_parallel: usize,
) -> Result<ArchiveTimings> {
    if packages.is_empty() {
        return Ok(ArchiveTimings {
            cache_hit_ms: 0,
            download_ms: 0,
            extract_ms: 0,
            overlap_ms: 0,
            link_ms: 0,
        });
    }

    let download_sema = Arc::new(Semaphore::new(http_parallel));
    let extract_sema = Arc::new(Semaphore::new(extract_concurrency()));
    let link_sema = Arc::new(Semaphore::new(link_concurrency()));
    let client = Arc::new(http_client().map_err(|e| Error::Message(e.to_string()))?);
    let auth = Arc::new(AuthStore::load_from_env().map_err(|e| Error::Message(e.to_string()))?);

    let download_sum_ns = Arc::new(AtomicU64::new(0));
    let extract_sum_ns = Arc::new(AtomicU64::new(0));
    let cache_hit_sum_ns = Arc::new(AtomicU64::new(0));
    let link_sum_ns = Arc::new(AtomicU64::new(0));
    let de_first_ns = Arc::new(AtomicU64::new(u64::MAX));
    let de_last_ns = Arc::new(AtomicU64::new(0));
    let link_first_ns = Arc::new(AtomicU64::new(u64::MAX));
    let link_last_ns = Arc::new(AtomicU64::new(0));
    let epoch = Instant::now();

    let mut set = JoinSet::new();
    for pkg in packages {
        let name = pkg.name.clone();
        let action = pkg.action;
        let url = pkg.dist_url.clone();
        let shasum = pkg.dist_shasum.clone();
        let dist_type = pkg.dist_type.clone();
        let store = store.clone();
        let vendor = vendor.to_path_buf();
        let download_sema = download_sema.clone();
        let extract_sema = extract_sema.clone();
        let link_sema = link_sema.clone();
        let client = client.clone();
        let auth = auth.clone();
        let download_sum_ns = download_sum_ns.clone();
        let extract_sum_ns = extract_sum_ns.clone();
        let cache_hit_sum_ns = cache_hit_sum_ns.clone();
        let link_sum_ns = link_sum_ns.clone();
        let de_first_ns = de_first_ns.clone();
        let de_last_ns = de_last_ns.clone();
        let link_first_ns = link_first_ns.clone();
        let link_last_ns = link_last_ns.clone();

        set.spawn(async move {
            pipeline_one(PipelineOne {
                store: &store,
                vendor: &vendor,
                name: &name,
                action,
                url: url.as_deref(),
                shasum: shasum.as_deref(),
                dist_type: dist_type.as_deref(),
                offline,
                download_sema: &download_sema,
                extract_sema: &extract_sema,
                link_sema: &link_sema,
                client: &client,
                auth: &auth,
                epoch,
                download_sum_ns: &download_sum_ns,
                extract_sum_ns: &extract_sum_ns,
                cache_hit_sum_ns: &cache_hit_sum_ns,
                link_sum_ns: &link_sum_ns,
                de_first_ns: &de_first_ns,
                de_last_ns: &de_last_ns,
                link_first_ns: &link_first_ns,
                link_last_ns: &link_last_ns,
            })
            .await
        });
    }

    while let Some(joined) = set.join_next().await {
        let (name, action) = joined.map_err(|e| Error::Message(format!("archive task: {e}")))??;
        eprintln!("puck: {} {name}", action_word(action));
    }

    let download_ms = Duration::from_nanos(download_sum_ns.load(Ordering::Relaxed)).as_millis();
    let extract_ms = Duration::from_nanos(extract_sum_ns.load(Ordering::Relaxed)).as_millis();
    let cache_hit_ms = Duration::from_nanos(cache_hit_sum_ns.load(Ordering::Relaxed)).as_millis();

    let de_first = de_first_ns.load(Ordering::Relaxed);
    let de_last = de_last_ns.load(Ordering::Relaxed);
    let de_wall_ms = if de_first == u64::MAX || de_last < de_first {
        0
    } else {
        Duration::from_nanos(de_last - de_first).as_millis()
    };
    let overlap_ms = (download_ms + extract_ms).saturating_sub(de_wall_ms);

    let link_first = link_first_ns.load(Ordering::Relaxed);
    let link_last = link_last_ns.load(Ordering::Relaxed);
    let link_ms = if link_first == u64::MAX || link_last < link_first {
        Duration::from_nanos(link_sum_ns.load(Ordering::Relaxed)).as_millis()
    } else {
        Duration::from_nanos(link_last - link_first).as_millis()
    };

    Ok(ArchiveTimings {
        cache_hit_ms,
        download_ms,
        extract_ms,
        overlap_ms,
        link_ms,
    })
}

struct PipelineOne<'a> {
    store: &'a Store,
    vendor: &'a Path,
    name: &'a str,
    action: InstallAction,
    url: Option<&'a str>,
    shasum: Option<&'a str>,
    dist_type: Option<&'a str>,
    offline: bool,
    download_sema: &'a Arc<Semaphore>,
    extract_sema: &'a Arc<Semaphore>,
    link_sema: &'a Arc<Semaphore>,
    client: &'a reqwest::Client,
    auth: &'a AuthStore,
    epoch: Instant,
    download_sum_ns: &'a AtomicU64,
    extract_sum_ns: &'a AtomicU64,
    cache_hit_sum_ns: &'a AtomicU64,
    link_sum_ns: &'a AtomicU64,
    de_first_ns: &'a AtomicU64,
    de_last_ns: &'a AtomicU64,
    link_first_ns: &'a AtomicU64,
    link_last_ns: &'a AtomicU64,
}

fn mark_window(first: &AtomicU64, last: &AtomicU64, epoch: Instant, start: Instant, end: Instant) {
    let start_ns = start.duration_since(epoch).as_nanos() as u64;
    let end_ns = end.duration_since(epoch).as_nanos() as u64;
    first.fetch_min(start_ns, Ordering::Relaxed);
    last.fetch_max(end_ns, Ordering::Relaxed);
}

async fn pipeline_one(p: PipelineOne<'_>) -> Result<(String, InstallAction)> {
    if p.dist_type
        .is_some_and(|t| t.eq_ignore_ascii_case("path"))
    {
        return Err(Error::Message(format!(
            "path package {} must be linked from dist.url (internal: skipped store fetch)",
            p.name
        )));
    }

    let url = p.url.ok_or_else(|| {
        Error::Message(format!(
            "package {} has no dist url (source installs not implemented yet)",
            p.name
        ))
    })?;

    // Store index short-circuit BEFORE any network / download permit.
    let lookup_started = Instant::now();
    if let Some(sha) =
        lookup(p.store, p.shasum, Some(url)).map_err(|e| Error::Message(e.to_string()))?
    {
        p.cache_hit_sum_ns.fetch_add(
            lookup_started.elapsed().as_nanos() as u64,
            Ordering::Relaxed,
        );
        eprintln!("puck: cache hit {}", p.name);
        link_package_now(&p, &sha).await?;
        return Ok((p.name.to_owned(), p.action));
    }

    if p.offline {
        return Err(Error::Message(format!(
            "offline install: {} is not in the warm store (no sha1/url index hit)",
            p.name
        )));
    }

    // Download under HTTP concurrency; release permit before extract.
    let dl_permit = p
        .download_sema
        .acquire()
        .await
        .map_err(|e| Error::Message(format!("download concurrency: {e}")))?;
    eprintln!("puck: downloading {}", p.name);
    let dl_started = Instant::now();
    let downloaded = download_with_client(p.client, url, p.shasum, p.auth)
        .await
        .map_err(|e| Error::Message(e.to_string()))?;
    let dl_ended = Instant::now();
    drop(dl_permit);
    p.download_sum_ns.fetch_add(
        dl_ended.duration_since(dl_started).as_nanos() as u64,
        Ordering::Relaxed,
    );
    mark_window(p.de_first_ns, p.de_last_ns, p.epoch, dl_started, dl_ended);

    let kind = ArchiveKind::from_type_and_url(p.dist_type, url)
        .map_err(|e| Error::Message(e.to_string()))?;
    let sha256 = downloaded.sha256.clone();
    let bytes = downloaded.bytes;
    let shasum = p.shasum.map(str::to_owned);
    let url_owned = url.to_owned();
    let store = p.store.clone();

    let ex_permit = p
        .extract_sema
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| Error::Message(format!("extract concurrency: {e}")))?;
    let ex_started = Instant::now();
    let sha = tokio::task::spawn_blocking(move || {
        let _permit = ex_permit;
        put_archive(&store, &sha256, &bytes, kind).map_err(|e| Error::Message(e.to_string()))?;
        remember(
            &store,
            &sha256,
            shasum.as_deref(),
            Some(url_owned.as_str()),
        )
        .map_err(|e| Error::Message(e.to_string()))?;
        Ok::<_, Error>(sha256)
    })
    .await
    .map_err(|e| Error::Message(format!("extract task: {e}")))??;
    let ex_ended = Instant::now();
    p.extract_sum_ns.fetch_add(
        ex_ended.duration_since(ex_started).as_nanos() as u64,
        Ordering::Relaxed,
    );
    mark_window(p.de_first_ns, p.de_last_ns, p.epoch, ex_started, ex_ended);

    link_package_now(&p, &sha).await?;
    Ok((p.name.to_owned(), p.action))
}

async fn link_package_now(p: &PipelineOne<'_>, sha: &str) -> Result<()> {
    let store_dir = p.store.package_dir(sha);
    let target = vendor_package_path(p.vendor, p.name);
    let link_permit = p
        .link_sema
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| Error::Message(format!("link concurrency: {e}")))?;
    let link_started = Instant::now();
    tokio::task::spawn_blocking(move || {
        let _permit = link_permit;
        link_one_package(&store_dir, &target)
    })
    .await
    .map_err(|e| Error::Message(format!("link task: {e}")))??;
    let link_ended = Instant::now();
    p.link_sum_ns.fetch_add(
        link_ended.duration_since(link_started).as_nanos() as u64,
        Ordering::Relaxed,
    );
    mark_window(
        p.link_first_ns,
        p.link_last_ns,
        p.epoch,
        link_started,
        link_ended,
    );
    Ok(())
}

fn action_word(action: InstallAction) -> &'static str {
    match action {
        InstallAction::Install => "installed",
        InstallAction::Update => "updated",
        InstallAction::Keep => "kept",
        InstallAction::Remove => "removed",
    }
}

fn vendor_package_path(vendor: &Path, name: &str) -> PathBuf {
    let mut path = vendor.to_path_buf();
    for part in name.split('/') {
        path.push(part);
    }
    path
}

fn link_one_package(store_dir: &Path, target: &Path) -> Result<()> {
    remove_vendor_path(target)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    fs::create_dir_all(target).map_err(|source| Error::Io {
        path: target.display().to_string(),
        source,
    })?;
    link_tree(store_dir, target).map_err(|e| Error::Message(e.to_string()))?;
    Ok(())
}

fn remove_package(vendor: &Path, name: &str) -> Result<()> {
    let target = vendor_package_path(vendor, name);
    if vendor_path_present(&target) {
        remove_vendor_path(&target)?;
        eprintln!("puck: removed {name}");
    }
    Ok(())
}

fn is_path_dist(pkg: &PlannedPackage) -> bool {
    pkg.dist_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("path"))
}

fn path_prefer_symlink(locked: &LockedPackage) -> bool {
    let opts = locked.extra.get("transport-options").or_else(|| {
        locked
            .dist
            .as_ref()
            .and_then(|d| d.extra.get("transport-options"))
    });
    match opts.and_then(|v| v.get("symlink")) {
        Some(Value::Bool(false)) => false,
        Some(Value::Number(n)) if n.as_u64() == Some(0) => false,
        Some(Value::String(s)) if s == "false" || s == "0" => false,
        _ => true,
    }
}

fn install_path_package(
    project_root: &Path,
    vendor: &Path,
    pkg: &PlannedPackage,
    locked: &LockedPackage,
) -> Result<()> {
    let rel = pkg
        .dist_url
        .as_deref()
        .ok_or_else(|| Error::Message(format!("path package {} has no dist url", pkg.name)))?;
    let source = {
        let p = Path::new(rel);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            project_root.join(p)
        }
    };
    if !source.exists() {
        return Err(Error::Message(format!(
            "path package {}: source {} does not exist",
            pkg.name,
            source.display()
        )));
    }

    let target = vendor_package_path(vendor, &pkg.name);
    remove_vendor_path(&target)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }

    if path_prefer_symlink(locked) {
        let link_value = relative_symlink_value(&target, &source);
        symlink_path(&link_value, &target)?;
    } else {
        copy_tree(&source, &target)?;
    }
    Ok(())
}

fn relative_symlink_value(link: &Path, source: &Path) -> PathBuf {
    let Some(parent) = link.parent() else {
        return source.to_path_buf();
    };
    let parent_abs = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    let source_abs = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    path_relative_to(&source_abs, &parent_abs).unwrap_or(source_abs)
}

fn path_relative_to(path: &Path, base: &Path) -> Option<PathBuf> {
    let path_c: Vec<_> = path.components().collect();
    let base_c: Vec<_> = base.components().collect();
    let common = path_c
        .iter()
        .zip(base_c.iter())
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return None;
    }
    let mut rel = PathBuf::new();
    for _ in common..base_c.len() {
        rel.push("..");
    }
    for c in &path_c[common..] {
        rel.push(c.as_os_str());
    }
    if rel.as_os_str().is_empty() {
        Some(PathBuf::from("."))
    } else {
        Some(rel)
    }
}

fn symlink_path(original: &Path, link: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link).map_err(|source| Error::Io {
            path: link.display().to_string(),
            source,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = original;
        Err(Error::Message(format!(
            "path package symlink is not supported on this platform ({})",
            link.display()
        )))
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(from).map_err(|source| Error::Io {
        path: from.display().to_string(),
        source,
    })?;
    if meta.file_type().is_dir() {
        fs::create_dir_all(to).map_err(|source| Error::Io {
            path: to.display().to_string(),
            source,
        })?;
        for entry in fs::read_dir(from).map_err(|source| Error::Io {
            path: from.display().to_string(),
            source,
        })? {
            let entry = entry.map_err(|source| Error::Io {
                path: from.display().to_string(),
                source,
            })?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else if meta.file_type().is_symlink() {
        let target = fs::read_link(from).map_err(|source| Error::Io {
            path: from.display().to_string(),
            source,
        })?;
        symlink_path(&target, to)
    } else {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        fs::copy(from, to).map_err(|source| Error::Io {
            path: to.display().to_string(),
            source,
        })?;
        Ok(())
    }
}

fn vendor_path_present(target: &Path) -> bool {
    target.exists() || fs::symlink_metadata(target).is_ok()
}

fn remove_vendor_path(target: &Path) -> Result<()> {
    let meta = match fs::symlink_metadata(target) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(Error::Io {
                path: target.display().to_string(),
                source,
            });
        }
    };
    if meta.file_type().is_symlink() || meta.file_type().is_file() {
        fs::remove_file(target).map_err(|source| Error::Io {
            path: target.display().to_string(),
            source,
        })?;
    } else {
        fs::remove_dir_all(target).map_err(|source| Error::Io {
            path: target.display().to_string(),
            source,
        })?;
    }
    Ok(())
}

fn write_installed_json(
    vendor: &Path,
    lock: &LockFile,
    plan: &InstallPlan,
    lock_by_name: &HashMap<String, &LockedPackage>,
    manifest: Option<&Manifest>,
    options: InstallOptions,
) -> Result<()> {
    let composer_dir = vendor.join("composer");
    fs::create_dir_all(&composer_dir).map_err(|source| Error::Io {
        path: composer_dir.display().to_string(),
        source,
    })?;

    let desired: Vec<&PlannedPackage> = plan
        .packages
        .iter()
        .filter(|p| p.action != InstallAction::Remove)
        .collect();

    let mut packages = Vec::new();
    let mut dev_names = Vec::new();
    for pkg in desired {
        let Some(locked) = lock_by_name.get(&pkg.name) else {
            continue;
        };
        let mut entry = locked_to_installed_value(locked);
        if pkg.is_dev {
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("dev".into(), Value::Bool(true));
            }
            dev_names.push(Value::String(locked.name.clone()));
        }
        packages.push(entry);
    }

    let dev_mode = !options.no_dev;
    let installed = json!({
        "packages": packages,
        "dev": dev_mode,
        "dev-package-names": dev_names,
    });

    let path = composer_dir.join("installed.json");
    let text =
        serde_json::to_string_pretty(&installed).map_err(|e| Error::Message(e.to_string()))?;
    fs::write(&path, text).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;

    let root = RootPackageMeta::from_manifest(manifest);
    let php = build_installed_php(lock, &plan.packages, &root, dev_mode)?;
    let php_path = composer_dir.join("installed.php");
    fs::write(&php_path, php).map_err(|source| Error::Io {
        path: php_path.display().to_string(),
        source,
    })?;

    Ok(())
}

fn locked_to_installed_value(pkg: &LockedPackage) -> Value {
    let mut map = Map::new();
    map.insert("name".into(), Value::String(pkg.name.clone()));
    map.insert("version".into(), Value::String(pkg.version.clone()));
    if let Some(dist) = &pkg.dist {
        let mut d = Map::new();
        if let Some(t) = &dist.dist_type {
            d.insert("type".into(), Value::String(t.clone()));
        }
        if let Some(u) = &dist.url {
            d.insert("url".into(), Value::String(u.clone()));
        }
        if let Some(r) = &dist.reference {
            d.insert("reference".into(), Value::String(r.clone()));
        }
        if let Some(s) = &dist.shasum {
            d.insert("shasum".into(), Value::String(s.clone()));
        }
        for (k, v) in &dist.extra {
            d.insert(k.clone(), v.clone());
        }
        map.insert("dist".into(), Value::Object(d));
    }
    let bins = pkg.bins();
    if !bins.is_empty() {
        map.insert(
            "bin".into(),
            Value::Array(bins.into_iter().map(Value::String).collect()),
        );
    }
    for (k, v) in &pkg.extra {
        if k == "bin" {
            continue;
        }
        map.insert(k.clone(), v.clone());
    }
    Value::Object(map)
}

#[cfg(test)]
mod resolve_parallel_tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn options_override_wins() {
        let forced = resolve_http_parallel(
            InstallOptions {
                http_parallel: Some(3),
                ..InstallOptions::default()
            },
            None,
        );
        assert_eq!(forced, 3);
    }

    #[test]
    fn reads_composer_config() {
        let m = Manifest::from_str(
            r#"{ "name": "acme/app", "config": { "max-parallel-http": 7 } }"#,
        )
        .expect("manifest");
        assert_eq!(
            resolve_http_parallel(InstallOptions::default(), Some(&m)),
            7
        );
    }
}

#[cfg(test)]
mod path_dist_tests {
    use super::*;
    use puck_lock::LockFile;
    use puck_store::Store;
    use std::path::PathBuf;
    use std::str::FromStr;

    fn path_lock(symlink: bool) -> LockFile {
        let transport = if symlink {
            r#""transport-options": { "symlink": true, "relative": true },"#
        } else {
            r#""transport-options": { "symlink": false },"#
        };
        let json = format!(
            r#"{{
            "content-hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "packages": [{{
                "name": "acme/hello",
                "version": "dev-main",
                "dist": {{ "type": "path", "url": "packages/acme-hello", "reference": "abc" }},
                {transport}
                "type": "library",
                "autoload": {{ "psr-4": {{ "Acme\\Hello\\": "src/" }} }}
            }}],
            "packages-dev": [],
            "aliases": [],
            "minimum-stability": "stable",
            "stability-flags": {{}},
            "prefer-stable": false,
            "prefer-lowest": false,
            "platform": {{}},
            "platform-dev": {{}},
            "plugin-api-version": "2.9.0"
        }}"#
        );
        LockFile::from_str(&json).expect("lock")
    }

    fn write_path_project(root: &Path, symlink: bool) -> LockFile {
        fs::create_dir_all(root.join("packages/acme-hello/src")).expect("dirs");
        fs::write(
            root.join("packages/acme-hello/composer.json"),
            r#"{"name":"acme/hello","type":"library","autoload":{"psr-4":{"Acme\\Hello\\":"src/"}}}"#,
        )
        .expect("pkg composer");
        fs::write(
            root.join("packages/acme-hello/src/Hello.php"),
            "<?php\nnamespace Acme\\Hello;\nclass Hello {}\n",
        )
        .expect("php");
        fs::write(
            root.join("composer.json"),
            r#"{"name":"puck/path-local","require":{"acme/hello":"*"},"repositories":[{"type":"path","url":"packages/acme-hello"}]}"#,
        )
        .expect("root composer");
        path_lock(symlink)
    }

    #[tokio::test]
    async fn path_dist_installs_symlink_by_default() {
        let tmp = tempfile::tempdir().expect("temp");
        let root = tmp.path();
        let lock = write_path_project(root, true);
        let plan = crate::plan_install(
            &lock,
            &crate::InstalledState::default(),
            InstallOptions::default(),
        )
        .expect("plan");
        let store_dir = tempfile::tempdir().expect("store");
        let store = Store::new(store_dir.path());
        execute_install(root, &lock, &plan, InstallOptions::default(), &store, None)
            .await
            .expect("install");
        let linked = root.join("vendor/acme/hello");
        assert!(
            linked
                .symlink_metadata()
                .expect("meta")
                .file_type()
                .is_symlink()
        );
        assert!(linked.join("composer.json").is_file());
        assert!(linked.join("src/Hello.php").is_file());
    }

    #[tokio::test]
    async fn path_dist_copies_when_symlink_false() {
        let tmp = tempfile::tempdir().expect("temp");
        let root = tmp.path();
        let lock = write_path_project(root, false);
        let plan = crate::plan_install(
            &lock,
            &crate::InstalledState::default(),
            InstallOptions::default(),
        )
        .expect("plan");
        let store_dir = tempfile::tempdir().expect("store");
        let store = Store::new(store_dir.path());
        execute_install(root, &lock, &plan, InstallOptions::default(), &store, None)
            .await
            .expect("install");
        let dest = root.join("vendor/acme/hello");
        let meta = dest.symlink_metadata().expect("meta");
        assert!(meta.is_dir());
        assert!(!meta.file_type().is_symlink());
        assert!(dest.join("src/Hello.php").is_file());
        // Copy is independent of source
        fs::remove_file(root.join("packages/acme-hello/src/Hello.php")).expect("rm src");
        assert!(dest.join("src/Hello.php").is_file());
    }

    #[tokio::test]
    async fn path_dist_remove_drops_symlink() {
        let tmp = tempfile::tempdir().expect("temp");
        let root = tmp.path();
        let lock = write_path_project(root, true);
        let plan = crate::plan_install(
            &lock,
            &crate::InstalledState::default(),
            InstallOptions::default(),
        )
        .expect("plan");
        let store_dir = tempfile::tempdir().expect("store");
        let store = Store::new(store_dir.path());
        execute_install(root, &lock, &plan, InstallOptions::default(), &store, None)
            .await
            .expect("install");
        let linked = root.join("vendor/acme/hello");
        assert!(linked.symlink_metadata().is_ok());
        remove_package(&root.join("vendor"), "acme/hello").expect("remove");
        assert!(!vendor_path_present(&linked));
        // Source package untouched
        assert!(root.join("packages/acme-hello/composer.json").is_file());
    }

    #[tokio::test]
    async fn fixture_path_local_installs_symlink() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/path-local");
        let tmp = tempfile::tempdir().expect("temp");
        // copy fixture tree
        let root = tmp.path();
        copy_tree(&fixture, root).expect("copy fixture");
        // remove vendor if any
        let _ = fs::remove_dir_all(root.join("vendor"));
        let lock = LockFile::from_path(root.join("composer.lock")).expect("lock");
        assert_eq!(
            lock.packages[0]
                .dist
                .as_ref()
                .and_then(|d| d.dist_type.as_deref()),
            Some("path"),
            "fixture dist.type"
        );
        let plan = crate::plan_install(
            &lock,
            &crate::InstalledState::default(),
            InstallOptions::default(),
        )
        .expect("plan");
        assert!(
            plan.packages[0]
                .dist_type
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case("path")),
            "planned dist_type={:?}",
            plan.packages[0].dist_type
        );
        let store_dir = tempfile::tempdir().expect("store");
        let store = Store::new(store_dir.path());
        execute_install(root, &lock, &plan, InstallOptions::default(), &store, None)
            .await
            .expect("install");
        let linked = root.join("vendor/acme/hello");
        assert!(
            linked
                .symlink_metadata()
                .expect("meta")
                .file_type()
                .is_symlink()
        );
    }
}
