//! Build an install plan from lock + installed state.

use crate::Result;
use crate::installed::InstalledState;
use indexmap::IndexMap;
use puck_lock::{LockFile, LockedPackage};
use std::path::{Path, PathBuf};

/// Options that affect which packages are planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InstallOptions {
    pub no_dev: bool,
    pub offline: bool,
}

/// What to do with a single package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallAction {
    Install,
    Update,
    Keep,
    Remove,
}

/// One package in the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedPackage {
    pub name: String,
    pub version: String,
    pub is_dev: bool,
    pub action: InstallAction,
    pub dist_url: Option<String>,
    pub dist_shasum: Option<String>,
    pub dist_type: Option<String>,
}

/// Full install plan.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstallPlan {
    pub packages: Vec<PlannedPackage>,
}

impl InstallPlan {
    pub fn to_install(&self) -> impl Iterator<Item = &PlannedPackage> {
        self.packages
            .iter()
            .filter(|p| matches!(p.action, InstallAction::Install | InstallAction::Update))
    }

    pub fn to_remove(&self) -> impl Iterator<Item = &PlannedPackage> {
        self.packages
            .iter()
            .filter(|p| p.action == InstallAction::Remove)
    }

    pub fn kept(&self) -> impl Iterator<Item = &PlannedPackage> {
        self.packages
            .iter()
            .filter(|p| p.action == InstallAction::Keep)
    }
}

/// Diff `lock` against `installed` and produce ordered actions.
///
/// Package order follows the lock file (`packages` then `packages-dev`).
/// Removals are appended for anything installed that is no longer desired.
pub fn plan_install(
    lock: &LockFile,
    installed: &InstalledState,
    options: InstallOptions,
) -> Result<InstallPlan> {
    let mut desired: IndexMap<String, (&LockedPackage, bool)> = IndexMap::new();
    for pkg in &lock.packages {
        desired.insert(pkg.name.to_ascii_lowercase(), (pkg, false));
    }
    if !options.no_dev {
        for pkg in &lock.packages_dev {
            desired.insert(pkg.name.to_ascii_lowercase(), (pkg, true));
        }
    }

    let mut packages = Vec::new();
    for (name, (pkg, is_dev)) in &desired {
        let action = match installed.packages.get(name) {
            Some(existing) if existing.version == pkg.version => InstallAction::Keep,
            Some(_) => InstallAction::Update,
            None => InstallAction::Install,
        };
        packages.push(planned_from_locked(pkg, *is_dev, action));
    }

    for (name, existing) in &installed.packages {
        if desired.contains_key(name) {
            continue;
        }
        // Installed but not desired: remove (including leftover require-dev when --no-dev).
        packages.push(PlannedPackage {
            name: name.clone(),
            version: existing.version.clone(),
            is_dev: existing.is_dev,
            action: InstallAction::Remove,
            dist_url: None,
            dist_shasum: None,
            dist_type: None,
        });
    }

    Ok(InstallPlan { packages })
}

/// If a Keep package's directory is missing under `vendor/`, promote it to Install.
pub fn reconcile_vendor_presence(plan: &mut InstallPlan, vendor_dir: &Path) {
    for pkg in &mut plan.packages {
        if pkg.action != InstallAction::Keep {
            continue;
        }
        if !vendor_package_dir(vendor_dir, &pkg.name).is_dir() {
            pkg.action = InstallAction::Install;
        }
    }
}

fn vendor_package_dir(vendor: &Path, name: &str) -> PathBuf {
    let mut path = vendor.to_path_buf();
    for part in name.split('/') {
        path.push(part);
    }
    path
}

fn planned_from_locked(pkg: &LockedPackage, is_dev: bool, action: InstallAction) -> PlannedPackage {
    PlannedPackage {
        name: pkg.name.to_ascii_lowercase(),
        version: pkg.version.clone(),
        is_dev,
        action,
        dist_url: pkg.dist.as_ref().and_then(|d| d.url.clone()),
        dist_shasum: pkg.dist.as_ref().and_then(|d| d.shasum.clone()),
        dist_type: pkg.dist.as_ref().and_then(|d| d.dist_type.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installed::InstalledPackage;
    use puck_lock::LockFile;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn laravel_lock() -> LockFile {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/laravel-skeleton/composer.lock");
        LockFile::from_path(&path).expect("laravel lock")
    }

    #[test]
    fn fresh_install_plans_all_packages() {
        let lock = laravel_lock();
        let plan = plan_install(&lock, &InstalledState::default(), InstallOptions::default())
            .expect("plan");
        let install_count = plan.to_install().count();
        assert_eq!(install_count, lock.packages.len() + lock.packages_dev.len());
        assert_eq!(plan.to_remove().count(), 0);
        assert_eq!(plan.kept().count(), 0);
    }

    #[test]
    fn no_dev_skips_dev_packages() {
        let lock = laravel_lock();
        let plan = plan_install(
            &lock,
            &InstalledState::default(),
            InstallOptions {
                no_dev: true,
                offline: false,
            },
        )
        .expect("plan");
        assert_eq!(plan.to_install().count(), lock.packages.len());
        assert!(plan.packages.iter().all(|p| !p.is_dev));
    }

    #[test]
    fn keeps_matching_versions() {
        let lock = laravel_lock();
        let first = &lock.packages[0];
        let mut packages = BTreeMap::new();
        packages.insert(
            first.name.to_ascii_lowercase(),
            InstalledPackage {
                name: first.name.to_ascii_lowercase(),
                version: first.version.clone(),
                is_dev: false,
            },
        );
        let installed = InstalledState { packages };
        let plan = plan_install(
            &lock,
            &installed,
            InstallOptions {
                no_dev: true,
                offline: false,
            },
        )
        .expect("plan");
        let kept = plan
            .kept()
            .find(|p| p.name == first.name.to_ascii_lowercase());
        assert!(kept.is_some());
        assert_eq!(plan.to_install().count(), lock.packages.len() - 1);
    }

    #[test]
    fn reconcile_promotes_keep_when_vendor_missing() {
        let mut plan = InstallPlan {
            packages: vec![PlannedPackage {
                name: "acme/lib".into(),
                version: "1.0.0".into(),
                is_dev: false,
                action: InstallAction::Keep,
                dist_url: None,
                dist_shasum: None,
                dist_type: None,
            }],
        };
        let tmp = tempfile::tempdir().expect("temp");
        reconcile_vendor_presence(&mut plan, &tmp.path().join("vendor"));
        assert_eq!(plan.packages[0].action, InstallAction::Install);
    }
}
