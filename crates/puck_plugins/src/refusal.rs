//! Tier 3: refuse allowed lock plugins that lack a native adapter.

/// Packages with a Tier 1 native adapter in `puck_plugins`.
pub const NATIVE_ADAPTERS: &[&str] = &[
    "pestphp/pest-plugin",
    "phpstan/extension-installer",
];

/// Package types that Composer treats as plugins (`composer-plugin` / legacy
/// `composer-installer`).
pub fn is_plugin_package_type(package_type: &str) -> bool {
    let t = package_type.to_ascii_lowercase();
    t == "composer-plugin" || t == "composer-installer"
}

/// Whether `name` has a shipped native adapter.
pub fn has_native_adapter(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    NATIVE_ADAPTERS.iter().any(|n| *n == name)
}

/// Allowed lock plugins with no native adapter.
///
/// `packages` yields `(name, package_type)` from the lock (packages + packages-dev).
/// `allows_plugin` mirrors `Manifest::allows_plugin`.
pub fn unsupported_allowed_plugins<I, F>(packages: I, allows_plugin: F) -> Vec<String>
where
    I: IntoIterator<Item = (String, String)>,
    F: Fn(&str) -> bool,
{
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (name, package_type) in packages {
        let name = name.to_ascii_lowercase();
        if !is_plugin_package_type(&package_type) {
            continue;
        }
        if has_native_adapter(&name) {
            continue;
        }
        if !allows_plugin(&name) {
            continue;
        }
        if seen.insert(name.clone()) {
            out.push(name);
        }
    }
    out
}

/// User-facing refusal text (plugin names + Composer fallback).
pub fn refuse_message(plugins: &[String]) -> String {
    debug_assert!(!plugins.is_empty());
    let list = plugins.join(", ");
    format!(
        "refused: allowed plugin(s) in the lock without a native adapter: {list}\n\
         use `composer install` for this project, or request a native adapter for the plugin(s)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_adapters_listed() {
        assert!(has_native_adapter("pestphp/pest-plugin"));
        assert!(has_native_adapter("PHPSTAN/extension-installer"));
        assert!(!has_native_adapter("php-http/discovery"));
    }

    #[test]
    fn ignores_non_plugins_and_disallowed() {
        let pkgs = vec![
            ("php-http/discovery".into(), "composer-plugin".into()),
            ("acme/lib".into(), "library".into()),
            ("pestphp/pest-plugin".into(), "composer-plugin".into()),
        ];
        let allows = |name: &str| name == "php-http/discovery" || name == "pestphp/pest-plugin";
        let unsupported = unsupported_allowed_plugins(pkgs, allows);
        assert_eq!(unsupported, vec!["php-http/discovery".to_string()]);
    }

    #[test]
    fn ignores_disallowed_plugins() {
        let pkgs = vec![("weird/plugin".into(), "composer-plugin".into())];
        let unsupported = unsupported_allowed_plugins(pkgs, |_| false);
        assert!(unsupported.is_empty());
    }

    #[test]
    fn treats_composer_installer_as_plugin() {
        let pkgs = vec![("old/installer".into(), "composer-installer".into())];
        let unsupported = unsupported_allowed_plugins(pkgs, |_| true);
        assert_eq!(unsupported, vec!["old/installer".to_string()]);
    }

    #[test]
    fn refuse_message_names_plugins_and_composer() {
        let msg = refuse_message(&["a/b".into(), "c/d".into()]);
        assert!(msg.contains("a/b"));
        assert!(msg.contains("c/d"));
        assert!(msg.contains("composer install"));
    }
}
