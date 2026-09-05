//! Platform package detection (`Composer\Repository\PlatformRepository` subset).

/// Returns true for `php`, `hhvm`, `ext-*`, `lib-*`, `composer-plugin-api`, etc.
pub fn is_platform_package(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "php"
        || name == "hhvm"
        || name == "composer"
        || name == "composer-plugin-api"
        || name == "composer-runtime-api"
        || name.starts_with("ext-")
        || name.starts_with("lib-")
        || name.starts_with("php-")
}
