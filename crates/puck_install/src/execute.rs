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
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const DEFAULT_FETCH_CONCURRENCY: usize = 8;

/// Run the plan against `project_root`.
pub async fn execute_install(
    project_root: &Path,
    lock: &LockFile,
    plan: &InstallPlan,
    options: InstallOptions,
    store: &Store,
    manifest: Option<&Manifest>,
) -> Result<()> {
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

    // Fetch + extract into the store in parallel.
    let to_fetch: Vec<&PlannedPackage> = plan.to_install().collect();
    let fetched = fetch_into_store(&to_fetch, store, options.offline).await?;

    // Link into vendor/ sequentially (filesystem-friendly).
    for pkg in &to_fetch {
        let Some(sha) = fetched.get(&pkg.name) else {
            return Err(Error::Message(format!(
                "missing store entry after fetch for {}",
                pkg.name
            )));
        };
        let store_dir = store.package_dir(sha);
        let target = vendor_package_path(&vendor, &pkg.name);
        if target.exists() {
            fs::remove_dir_all(&target).map_err(|source| Error::Io {
                path: target.display().to_string(),
                source,
            })?;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        fs::create_dir_all(&target).map_err(|source| Error::Io {
            path: target.display().to_string(),
            source,
        })?;
        link_tree(&store_dir, &target).map_err(|e| Error::Message(e.to_string()))?;
        eprintln!("puck: {} {}", action_word(pkg.action), pkg.name);
    }

    write_installed_json(&vendor, lock, plan, &lock_by_name, manifest, options)?;
    crate::bins::install_binaries(project_root, lock, options.no_dev)?;
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

fn remove_package(vendor: &Path, name: &str) -> Result<()> {
    let target = vendor_package_path(vendor, name);
    if target.exists() {
        fs::remove_dir_all(&target).map_err(|source| Error::Io {
            path: target.display().to_string(),
            source,
        })?;
        eprintln!("puck: removed {name}");
    }
    Ok(())
}

async fn fetch_into_store(
    packages: &[&PlannedPackage],
    store: &Store,
    offline: bool,
) -> Result<HashMap<String, String>> {
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
            let result = fetch_one(
                &store,
                &name,
                url.as_deref(),
                shasum.as_deref(),
                dist_type.as_deref(),
                offline,
            )
            .await;
            (name, result)
        });
    }

    let mut out = HashMap::new();
    while let Some(joined) = set.join_next().await {
        let (name, result) = joined.map_err(|e| Error::Message(format!("fetch task: {e}")))?;
        let sha = result?;
        out.insert(name, sha);
    }
    Ok(out)
}

async fn fetch_one(
    store: &Store,
    name: &str,
    url: Option<&str>,
    shasum: Option<&str>,
    dist_type: Option<&str>,
    offline: bool,
) -> Result<String> {
    let url = url.ok_or_else(|| {
        Error::Message(format!(
            "package {name} has no dist url (source installs not implemented yet)"
        ))
    })?;

    if let Some(sha) = lookup(store, shasum, Some(url)).map_err(|e| Error::Message(e.to_string()))?
    {
        eprintln!("puck: cache hit {name}");
        return Ok(sha);
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
    Ok(downloaded.sha256)
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
