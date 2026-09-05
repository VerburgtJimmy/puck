//! Execute an install plan: fetch, store, link, write installed metadata.

use crate::installed_php::{RootPackageMeta, build_installed_php};
use crate::plan::{InstallAction, InstallOptions, InstallPlan, PlannedPackage};
use crate::{Error, Result};
use puck_dist::{ArchiveKind, download};
use puck_lock::{LockFile, LockedPackage};
use puck_manifest::Manifest;
use puck_store::{Store, link_tree, lookup, put_archive, remember};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const DEFAULT_FETCH_CONCURRENCY: usize = 8;

/// Link concurrency: hardlinks are metadata-heavy; prefer CPU count (min 8)
/// so warm-wipe can saturate APFS better than the fetch default alone.
fn link_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(DEFAULT_FETCH_CONCURRENCY)
        .max(DEFAULT_FETCH_CONCURRENCY)
}

/// Wall-clock phase timings from [`execute_install`] (milliseconds).
#[derive(Debug, Clone, Default)]
pub struct ExecuteTimings {
    /// Store lookup / download / extract (parallel wall clock).
    pub fetch_ms: u128,
    /// Sum of per-package cache-hit lookup times (may exceed [`Self::fetch_ms`] under concurrency).
    pub fetch_cache_hit_ms: u128,
    /// Sum of per-package download+extract times (may exceed [`Self::fetch_ms`] under concurrency).
    pub fetch_download_ms: u128,
    /// Vendor hardlink/copy phase.
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
    let link_started = Instant::now();
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

    // Fetch + extract archives into the store in parallel.
    let fetch_started = Instant::now();
    let (fetched, fetch_cache_hit_ms, fetch_download_ms) =
        fetch_into_store(&archive_pkgs, store, options.offline).await?;
    let fetch_ms = fetch_started.elapsed().as_millis();

    // Link into vendor/ in parallel. Hardlinks are mostly metadata; serial
    // linking was ~70% of warm-wipe wall time. Use at least fetch concurrency,
    // scaled up to available parallelism when the host has more cores.
    let link_sema = Arc::new(Semaphore::new(link_concurrency()));
    let mut link_set = JoinSet::new();
    for pkg in &archive_pkgs {
        let Some(sha) = fetched.get(&pkg.name) else {
            return Err(Error::Message(format!(
                "missing store entry after fetch for {}",
                pkg.name
            )));
        };
        let name = pkg.name.clone();
        let action = pkg.action;
        let store_dir = store.package_dir(sha);
        let target = vendor_package_path(&vendor, &name);
        let permit = link_sema
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| Error::Message(format!("link concurrency: {e}")))?;
        link_set.spawn_blocking(move || {
            let _permit = permit;
            link_one_package(&store_dir, &target)?;
            Ok::<_, Error>((name, action))
        });
    }
    while let Some(joined) = link_set.join_next().await {
        let (name, action) =
            joined.map_err(|e| Error::Message(format!("link task: {e}")))??;
        eprintln!("puck: {} {name}", action_word(action));
    }
    let link_ms = link_started.elapsed().as_millis();

    let meta_started = Instant::now();
    write_installed_json(&vendor, lock, plan, &lock_by_name, manifest, options)?;
    crate::bins::install_binaries(project_root, lock, options.no_dev)?;
    let installed_meta_ms = meta_started.elapsed().as_millis();

    Ok(ExecuteTimings {
        fetch_ms,
        fetch_cache_hit_ms,
        fetch_download_ms,
        link_ms,
        installed_meta_ms,
    })
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
    let opts = locked
        .extra
        .get("transport-options")
        .or_else(|| {
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
    let rel = pkg.dist_url.as_deref().ok_or_else(|| {
        Error::Message(format!(
            "path package {} has no dist url",
            pkg.name
        ))
    })?;
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

enum FetchKind {
    CacheHit,
    Download,
}

async fn fetch_into_store(
    packages: &[&PlannedPackage],
    store: &Store,
    offline: bool,
) -> Result<(HashMap<String, String>, u128, u128)> {
    let semaphore = Arc::new(Semaphore::new(DEFAULT_FETCH_CONCURRENCY));
    let mut set = JoinSet::new();

    for pkg in packages {
        let name = pkg.name.clone();
        let url = pkg.dist_url.clone();
        let shasum = pkg.dist_shasum.clone();
        let dist_type = pkg.dist_type.clone();
        let store = store.clone();
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| Error::Message(format!("concurrency permit: {e}")))?;

        set.spawn(async move {
            let _permit = permit;
            let started = Instant::now();
            let result = fetch_one(
                &store,
                &name,
                url.as_deref(),
                shasum.as_deref(),
                dist_type.as_deref(),
                offline,
            )
            .await;
            let elapsed = started.elapsed();
            (name, result, elapsed)
        });
    }

    let mut out = HashMap::new();
    let mut cache_hit = Duration::ZERO;
    let mut download = Duration::ZERO;
    while let Some(joined) = set.join_next().await {
        let (name, result, elapsed) =
            joined.map_err(|e| Error::Message(format!("fetch task: {e}")))?;
        let (sha, kind) = result?;
        match kind {
            FetchKind::CacheHit => cache_hit += elapsed,
            FetchKind::Download => download += elapsed,
        }
        out.insert(name, sha);
    }
    Ok((out, cache_hit.as_millis(), download.as_millis()))
}

async fn fetch_one(
    store: &Store,
    name: &str,
    url: Option<&str>,
    shasum: Option<&str>,
    dist_type: Option<&str>,
    offline: bool,
) -> Result<(String, FetchKind)> {
    if dist_type.is_some_and(|t| t.eq_ignore_ascii_case("path")) {
        return Err(Error::Message(format!(
            "path package {name} must be linked from dist.url (internal: skipped store fetch)"
        )));
    }

    let url = url.ok_or_else(|| {
        Error::Message(format!(
            "package {name} has no dist url (source installs not implemented yet)"
        ))
    })?;

    if let Some(sha) = lookup(store, shasum, Some(url)).map_err(|e| Error::Message(e.to_string()))?
    {
        eprintln!("puck: cache hit {name}");
        return Ok((sha, FetchKind::CacheHit));
    }

    if offline {
        return Err(Error::Message(format!(
            "offline install: {name} is not in the warm store (no sha1/url index hit)"
        )));
    }

    eprintln!("puck: downloading {name}");
    let downloaded = download(url, shasum)
        .await
        .map_err(|e| Error::Message(e.to_string()))?;
    let kind = ArchiveKind::from_type_and_url(dist_type, url)
        .map_err(|e| Error::Message(e.to_string()))?;
    put_archive(store, &downloaded.sha256, &downloaded.bytes, kind)
        .map_err(|e| Error::Message(e.to_string()))?;
    remember(store, &downloaded.sha256, shasum, Some(url))
        .map_err(|e| Error::Message(e.to_string()))?;
    Ok((downloaded.sha256, FetchKind::Download))
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
        assert!(linked.symlink_metadata().expect("meta").file_type().is_symlink());
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
            lock.packages[0].dist.as_ref().and_then(|d| d.dist_type.as_deref()),
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
        assert!(linked.symlink_metadata().expect("meta").file_type().is_symlink());
    }
}
