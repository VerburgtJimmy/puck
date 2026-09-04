//! Classmap scanning (Composer ClassMapGenerator).
//!
//! Declared `autoload.classmap` paths are always scanned. With `-o` / optimize,
//! PSR-0 and PSR-4 directories are scanned too. On name collisions, first-wins.

use crate::collect::RelPath;
use regex::Regex;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const INSTALLED_VERSIONS_CLASS: &str = "Composer\\InstalledVersions";
const INSTALLED_VERSIONS_PATH: &str = "vendor/composer/InstalledVersions.php";

// Rust `regex` has no lookaround; approximate Composer’s `(?<![\\$:>])` by
// requiring a non-special char (or start) before the keyword.
static CLASS_OR_NS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?xi)
        (?:
            (?:^|[^\\$:>])\b(?P<type>class|interface|trait|enum)\s+(?P<name>[a-zA-Z_\x7f-\xff:][a-zA-Z0-9_\x7f-\xff:\-]*)
          | (?:^|[^\\$:>])\b(?P<ns>namespace)(?P<nsname>\s+[a-zA-Z_\x7f-\xff][a-zA-Z0-9_\x7f-\xff]*(?:\s*\\\s*[a-zA-Z_\x7f-\xff][a-zA-Z0-9_\x7f-\xff]*)*)?\s*[\{;]
        )
        ",
    )
    .expect("classmap regex")
});

/// Scan classmap paths under `project_root` into FQCN -> project-relative path.
///
/// `declared` is always scanned. `extra` is used for `-o` PSR directory scans.
/// Always includes [`INSTALLED_VERSIONS_CLASS`]. Missing paths are skipped.
pub fn build_classmap(
    project_root: &Path,
    declared: &[RelPath],
    extra: &[RelPath],
) -> BTreeMap<String, RelPath> {
    let mut map = BTreeMap::new();

    for rel in declared.iter().chain(extra.iter()) {
        let abs = project_root.join(rel);
        if !abs.exists() {
            continue;
        }
        scan_entry(project_root, &abs, &mut map);
    }

    // Composer always sets InstalledVersions (overwrites any earlier hit), then ksorts.
    map.insert(
        INSTALLED_VERSIONS_CLASS.to_owned(),
        INSTALLED_VERSIONS_PATH.to_owned(),
    );

    map
}

/// Flatten PSR-4 / PSR-0 path lists for optimize scans (deduped, stable order).
pub fn psr_scan_paths(
    psr4: &[(&str, &Vec<RelPath>)],
    psr0: &[(&str, &Vec<RelPath>)],
) -> Vec<RelPath> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for (_, paths) in psr4.iter().chain(psr0.iter()) {
        for path in *paths {
            if seen.insert(path.clone()) {
                out.push(path.clone());
            }
        }
    }
    out
}

fn scan_entry(project_root: &Path, path: &Path, map: &mut BTreeMap<String, RelPath>) {
    if is_excluded(path) {
        return;
    }
    if path.is_file() {
        if is_php_like(path) {
            scan_file(project_root, path, map);
        }
        return;
    }
    if !path.is_dir() {
        return;
    }

    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| !is_excluded(p))
            .collect();
        children.sort();
        for child in children {
            if child.is_dir() {
                stack.push(child);
            } else if child.is_file() && is_php_like(&child) {
                scan_file(project_root, &child, map);
            }
        }
    }
}

fn is_excluded(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == ".puck-ok")
}

fn is_php_like(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("php" | "hh")
    )
}

fn scan_file(project_root: &Path, path: &Path, map: &mut BTreeMap<String, RelPath>) {
    let Ok(contents) = fs::read_to_string(path) else {
        return;
    };
    let Some(rel) = to_rel_path(project_root, path) else {
        return;
    };
    for class in find_classes(&contents) {
        // First-wins (Composer ClassMapGenerator).
        map.entry(class).or_insert_with(|| rel.clone());
    }
}

fn to_rel_path(project_root: &Path, path: &Path) -> Option<RelPath> {
    let rel = path.strip_prefix(project_root).ok()?;
    let mut s = rel.to_string_lossy().replace('\\', "/");
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    Some(s)
}

/// Extract top-level class / interface / trait / enum FQCNs from PHP source.
pub fn find_classes(contents: &str) -> Vec<String> {
    let cleaned = strip_php_noise(contents);
    if !CLASS_OR_NS.is_match(&cleaned) {
        return Vec::new();
    }

    let mut classes = Vec::new();
    let mut namespace = String::new();

    for caps in CLASS_OR_NS.captures_iter(&cleaned) {
        if caps.name("ns").is_some() {
            let nsname = caps.name("nsname").map(|m| m.as_str()).unwrap_or("");
            namespace = nsname
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            if !namespace.is_empty() && !namespace.ends_with('\\') {
                namespace.push('\\');
            }
            continue;
        }

        let Some(name_m) = caps.name("name") else {
            continue;
        };
        let mut name = name_m.as_str().to_owned();
        // Anonymous class: `new class extends|implements ...`
        if name == "extends" || name == "implements" {
            continue;
        }

        let type_name = caps
            .name("type")
            .map(|m| m.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if name.starts_with(':') {
            // XHP: `:foo:bar` -> `xhp_foo__bar`
            name = format!(
                "xhp{}",
                name[1..].replace('-', "_").replace(':', "__")
            );
        } else if type_name == "enum"
            && let Some(pos) = name.rfind(':')
        {
            name.truncate(pos);
        }

        let fqcn = format!("{namespace}{name}");
        let fqcn = fqcn.trim_start_matches('\\').to_owned();
        if !fqcn.is_empty() {
            classes.push(fqcn);
        }
    }

    classes
}

/// Strip comments and string literals so keywords inside them are ignored.
fn strip_php_noise(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let len = bytes.len();

    // Drop leading non-PHP (Composer strips until `<?`).
    if let Some(pos) = src.find("<?") {
        i = pos;
        if src[i..].starts_with("<?php") {
            i += 5;
        } else if src[i..].starts_with("<?=") {
            i += 3;
        } else {
            i += 2;
        }
    }

    while i < len {
        let b = bytes[i];
        // Line comment //
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Line comment #
        if b == b'#' {
            i += 1;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
            out.push(' ');
            continue;
        }
        // Single-quoted string
        if b == b'\'' {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                    continue;
                }
                if bytes[i] == b'\'' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(' ');
            continue;
        }
        // Double-quoted string
        if b == b'"' {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(' ');
            continue;
        }
        // Heredoc / nowdoc: <<<
        if b == b'<' && i + 2 < len && bytes[i + 1] == b'<' && bytes[i + 2] == b'<' {
            i += 3;
            while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            let nowdoc = i < len && bytes[i] == b'\'';
            if nowdoc {
                i += 1;
            }
            let label_start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let label = &src[label_start..i];
            if nowdoc && i < len && bytes[i] == b'\'' {
                i += 1;
            }
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            if i < len {
                i += 1; // newline after label
            }
            let end_pat = format!("\n{label}");
            if let Some(rel) = src[i..].find(&end_pat) {
                i += rel + end_pat.len();
                while i < len && bytes[i] != b'\n' && bytes[i] != b';' {
                    i += 1;
                }
                if i < len && bytes[i] == b';' {
                    i += 1;
                }
            } else {
                i = len;
            }
            out.push(' ');
            continue;
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn find_classes_simple_namespaced() {
        let src = "<?php\nnamespace Acme;\nclass Foo {}\n";
        assert_eq!(find_classes(src), vec!["Acme\\Foo".to_owned()]);
    }

    #[test]
    fn find_classes_skips_anonymous() {
        let src = "<?php\nnamespace Acme;\n$x = new class extends \\stdClass {};\nclass Bar {}\n";
        assert_eq!(find_classes(src), vec!["Acme\\Bar".to_owned()]);
    }

    #[test]
    fn find_classes_interface_trait_enum() {
        let src = r#"<?php
namespace Acme;
interface I {}
trait T {}
enum E: string { case A = 'a'; }
"#;
        let found = find_classes(src);
        assert!(found.contains(&"Acme\\I".to_owned()));
        assert!(found.contains(&"Acme\\T".to_owned()));
        assert!(found.contains(&"Acme\\E".to_owned()));
    }

    #[test]
    fn build_classmap_scans_src() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("src");
        fs::create_dir_all(&src).expect("mkdir");
        fs::write(src.join("Foo.php"), "<?php\nnamespace Acme;\nclass Foo {}\n")
            .expect("write");

        let map = build_classmap(dir.path(), &["src".to_owned()], &[]);
        assert_eq!(
            map.get("Acme\\Foo").map(String::as_str),
            Some("src/Foo.php")
        );
        assert_eq!(
            map.get(INSTALLED_VERSIONS_CLASS).map(String::as_str),
            Some(INSTALLED_VERSIONS_PATH)
        );
    }

    #[test]
    fn first_wins_on_collision() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(&a).expect("mkdir a");
        fs::create_dir_all(&b).expect("mkdir b");
        fs::write(a.join("Foo.php"), "<?php\nclass Foo {}\n").expect("write a");
        fs::write(b.join("Foo.php"), "<?php\nclass Foo {}\n").expect("write b");

        let map = build_classmap(dir.path(), &["a".to_owned(), "b".to_owned()], &[]);
        assert_eq!(map.get("Foo").map(String::as_str), Some("a/Foo.php"));
    }

    #[test]
    fn excludes_puck_ok() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("src");
        fs::create_dir_all(&src).expect("mkdir");
        fs::write(src.join("Foo.php"), "<?php\nclass Foo {}\n").expect("write");
        fs::write(src.join(".puck-ok"), "ok").expect("marker");

        let map = build_classmap(dir.path(), &["src".to_owned()], &[]);
        assert!(map.contains_key("Foo"));
        // marker is not a php file; ensure we did not error walking past it
        assert!(!map.values().any(|p| p.contains(".puck-ok")));
    }
}
