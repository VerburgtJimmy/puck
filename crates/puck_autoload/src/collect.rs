//! Collect PSR-4 / PSR-0 / files / classmap rules from lock + root manifest.

use crate::php;
use indexmap::IndexMap;
use puck_lock::{LockFile, LockedPackage};
use puck_manifest::{Autoload, Manifest};
use serde_json::Value;
use std::collections::BTreeMap;

/// Relative path from project root (e.g. `vendor/foo/bar/src` or `app`).
pub type RelPath = String;

#[derive(Debug, Default)]
pub struct CollectedAutoloads {
    /// Namespace -> paths (PSR-4). Keys later sorted with krsort.
    pub psr4: IndexMap<String, Vec<RelPath>>,
    /// Namespace -> paths (PSR-0).
    pub psr0: IndexMap<String, Vec<RelPath>>,
    /// file-id -> relative path (project-root relative before path-code).
    pub files: IndexMap<String, RelPath>,
    /// Declared classmap dirs/files (scanned during dump).
    pub classmap: Vec<RelPath>,
}

impl CollectedAutoloads {
    pub fn from_lock_and_manifest(
        lock: &LockFile,
        manifest: Option<&Manifest>,
        no_dev: bool,
    ) -> Self {
        let mut out = Self::default();

        let packages: Vec<&LockedPackage> = if no_dev {
            lock.packages.iter().collect()
        } else {
            lock.packages
                .iter()
                .chain(lock.packages_dev.iter())
                .collect()
        };

        // Root first for PSR maps (Composer packageMap order), packages in lock order.
        if let Some(m) = manifest {
            merge_manifest_autoload(&mut out, m, no_dev);
        }

        for pkg in &packages {
            if let Some(autoload) = pkg.extra.get("autoload") {
                merge_json_autoload(
                    &mut out,
                    &pkg.name,
                    autoload,
                    &vendor_install_path(&pkg.name),
                );
            }
        }

        // Files: Composer sorts by dependency weight then appends root last.
        // M1: re-collect files with packages first, root last.
        out.files.clear();
        for pkg in &packages {
            if let Some(autoload) = pkg.extra.get("autoload") {
                append_files(
                    &mut out.files,
                    &pkg.name,
                    autoload,
                    &vendor_install_path(&pkg.name),
                );
            }
        }
        if let Some(m) = manifest {
            append_manifest_files(&mut out.files, m, no_dev);
        }

        out
    }

    /// PSR-4 entries in Composer `krsort` order.
    pub fn psr4_sorted(&self) -> Vec<(&str, &Vec<RelPath>)> {
        krsort_map(&self.psr4)
    }

    pub fn psr0_sorted(&self) -> Vec<(&str, &Vec<RelPath>)> {
        krsort_map(&self.psr0)
    }
}

fn krsort_map(map: &IndexMap<String, Vec<RelPath>>) -> Vec<(&str, &Vec<RelPath>)> {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by(|a, b| b.cmp(a));
    keys.into_iter()
        .filter_map(|k| map.get(k).map(|v| (k.as_str(), v)))
        .collect()
}

fn vendor_install_path(name: &str) -> String {
    format!("vendor/{name}")
}

fn merge_manifest_autoload(out: &mut CollectedAutoloads, m: &Manifest, no_dev: bool) {
    if let Some(a) = &m.autoload {
        merge_typed_autoload(out, &m.name, a, "");
    }
    if !no_dev && let Some(a) = &m.autoload_dev {
        merge_typed_autoload(out, &m.name, a, "");
    }
}

fn merge_typed_autoload(
    out: &mut CollectedAutoloads,
    package_name: &str,
    a: &Autoload,
    install_path: &str,
) {
    for (ns, paths) in &a.psr4 {
        let ns = normalize_psr_namespace(ns);
        for path in value_paths(paths) {
            let rel = join_install(install_path, &path);
            out.psr4.entry(ns.clone()).or_default().push(rel);
        }
    }
    for (ns, paths) in &a.psr0 {
        let ns = normalize_psr_namespace(ns);
        for path in value_paths(paths) {
            let rel = join_install(install_path, &path);
            out.psr0.entry(ns.clone()).or_default().push(rel);
        }
    }
    for path in &a.classmap {
        if let Some(p) = value_as_path(path) {
            out.classmap.push(join_install(install_path, &p));
        }
    }
    // files handled separately for ordering
    let _ = package_name;
}

fn merge_json_autoload(
    out: &mut CollectedAutoloads,
    package_name: &str,
    autoload: &Value,
    install_path: &str,
) {
    let Some(obj) = autoload.as_object() else {
        return;
    };

    if let Some(psr4) = obj.get("psr-4").and_then(Value::as_object) {
        for (ns, paths) in psr4 {
            let ns = normalize_psr_namespace(ns);
            for path in value_paths(paths) {
                out.psr4
                    .entry(ns.clone())
                    .or_default()
                    .push(join_install(install_path, &path));
            }
        }
    }
    if let Some(psr0) = obj.get("psr-0").and_then(Value::as_object) {
        for (ns, paths) in psr0 {
            let ns = normalize_psr_namespace(ns);
            for path in value_paths(paths) {
                out.psr0
                    .entry(ns.clone())
                    .or_default()
                    .push(join_install(install_path, &path));
            }
        }
    }
    if let Some(classmap) = obj.get("classmap").and_then(Value::as_array) {
        for path in classmap {
            if let Some(p) = value_as_path(path) {
                out.classmap.push(join_install(install_path, &p));
            }
        }
    }
    let _ = package_name;
}

fn append_files(
    files: &mut IndexMap<String, RelPath>,
    package_name: &str,
    autoload: &Value,
    install_path: &str,
) {
    let Some(arr) = autoload
        .as_object()
        .and_then(|o| o.get("files"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for path in arr {
        let Some(p) = value_as_path(path) else {
            continue;
        };
        let id = file_identifier(package_name, &p);
        let rel = join_install(install_path, &p);
        files.insert(id, rel);
    }
}

fn append_manifest_files(files: &mut IndexMap<String, RelPath>, m: &Manifest, no_dev: bool) {
    if let Some(a) = &m.autoload {
        for path in &a.files {
            let Some(p) = value_as_path(path) else {
                continue;
            };
            let id = file_identifier(&m.name, &p);
            files.insert(id, join_install("", &p));
        }
    }
    if !no_dev && let Some(a) = &m.autoload_dev {
        for path in &a.files {
            let Some(p) = value_as_path(path) else {
                continue;
            };
            let id = file_identifier(&m.name, &p);
            files.insert(id, join_install("", &p));
        }
    }
}

/// Composer: `md5($package->getName() . ':' . $path)` where `$path` is the
/// autoload rule path (not the install-prefixed path).
pub fn file_identifier(package_name: &str, path: &str) -> String {
    php::md5_hex(format!("{package_name}:{path}").as_bytes())
}

fn normalize_psr_namespace(ns: &str) -> String {
    ns.trim_start_matches('\\').to_owned()
}

fn value_paths(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items.iter().filter_map(value_as_path).collect(),
        _ => Vec::new(),
    }
}

fn value_as_path(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

fn join_install(install_path: &str, path: &str) -> String {
    let path = path.replace('\\', "/");
    let joined = if install_path.is_empty() {
        if path.is_empty() {
            ".".to_owned()
        } else {
            path
        }
    } else if path.is_empty() {
        install_path.to_owned()
    } else {
        format!("{install_path}/{path}")
    };
    normalize_rel_path(&joined)
}

fn normalize_rel_path(path: &str) -> String {
    let mut p = path.replace('\\', "/");
    while p.contains("//") {
        p = p.replace("//", "/");
    }
    while p.ends_with('/') && p != "/" && p != "." {
        p.pop();
    }
    p
}

/// Group PSR-4 namespaces by first character for `prefixLengthsPsr4`.
pub fn prefix_lengths_psr4(
    psr4: &[(&str, &Vec<RelPath>)],
) -> BTreeMap<char, BTreeMap<String, usize>> {
    let mut out: BTreeMap<char, BTreeMap<String, usize>> = BTreeMap::new();
    for (ns, _) in psr4 {
        let first = ns.chars().next().unwrap_or('\\');
        out.entry(first)
            .or_default()
            .insert((*ns).to_owned(), ns.len());
    }
    out
}

/// Group PSR-0 for `prefixesPsr0` (first char -> ns -> paths).
pub fn prefixes_psr0(
    psr0: &[(&str, &Vec<RelPath>)],
) -> BTreeMap<char, IndexMap<String, Vec<RelPath>>> {
    let mut out: BTreeMap<char, IndexMap<String, Vec<RelPath>>> = BTreeMap::new();
    for (ns, paths) in psr0 {
        let first = ns.chars().next().unwrap_or('\\');
        out.entry(first)
            .or_default()
            .insert((*ns).to_owned(), (*paths).clone());
    }
    out
}
