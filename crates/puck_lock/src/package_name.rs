//! Composer package-name validation.
//!
//! Matches Packagist's / Composer's name rule:
//! `^[a-z0-9]([_.-]?[a-z0-9]+)*/[a-z0-9](([_.]?|-{0,2})[a-z0-9]+)*$`

use regex::Regex;
use std::sync::OnceLock;

fn package_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[a-z0-9]([_.-]?[a-z0-9]+)*/[a-z0-9](([_.]?|-{0,2})[a-z0-9]+)*$")
            .expect("package name regex")
    })
}

/// True when `name` is a valid Composer `vendor/package` name.
pub fn is_valid_package_name(name: &str) -> bool {
    package_name_re().is_match(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_common_names() {
        assert!(is_valid_package_name("laravel/framework"));
        assert!(is_valid_package_name("symfony/http-foundation"));
        assert!(is_valid_package_name("a/b"));
        assert!(is_valid_package_name("foo_bar/baz.qux"));
    }

    #[test]
    fn rejects_traversal_and_junk() {
        assert!(!is_valid_package_name("../.ssh"));
        assert!(!is_valid_package_name("../../etc/passwd"));
        assert!(!is_valid_package_name("Foo/Bar"));
        assert!(!is_valid_package_name("only-one-segment"));
        assert!(!is_valid_package_name("a/b/c"));
        assert!(!is_valid_package_name("/a/b"));
        assert!(!is_valid_package_name(""));
    }
}
