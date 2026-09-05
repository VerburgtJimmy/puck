//! Stability acceptance (`Composer\Package\Version\StabilityFilter`).

use crate::package::Package;
use indexmap::IndexMap;
use puck_version::{parse_stability, Stability};

/// `StabilityFilter::isPackageAcceptable`.
///
/// A package is acceptable when, for any of its names (including provide/replace),
/// either a per-name stability flag allows its stability, or the package stability
/// is at most `minimum_stability`.
pub fn is_package_acceptable(
    package: &Package,
    minimum_stability: Stability,
    stability_flags: &IndexMap<String, Stability>,
) -> bool {
    let stability = parse_stability(&package.version);
    for name in package.names(true) {
        if let Some(&flag) = stability_flags.get(&name) {
            if stability <= flag {
                return true;
            }
        } else if stability <= minimum_stability {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Package;

    #[test]
    fn stable_min_rejects_dev() {
        let pkg = Package::new("a/a", "1.0.0.0-dev", "1.0-dev");
        assert!(!is_package_acceptable(&pkg, Stability::Stable, &IndexMap::new()));
    }

    #[test]
    fn stable_min_accepts_stable() {
        let pkg = Package::new("a/a", "1.0.0.0", "1.0");
        assert!(is_package_acceptable(&pkg, Stability::Stable, &IndexMap::new()));
    }

    #[test]
    fn flag_allows_dev_for_named_package() {
        let pkg = Package::new("c/c", "2.0.0.0-dev", "2.0-dev");
        let mut flags = IndexMap::new();
        flags.insert("c/c".into(), Stability::Dev);
        assert!(is_package_acceptable(&pkg, Stability::Stable, &flags));
    }
}
