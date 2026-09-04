//! Small PHP / hashing helpers for generated autoload files.

pub fn md5_hex(bytes: &[u8]) -> String {
    use md5::{Digest, Md5};
    let digest = Md5::digest(bytes);
    let mut out = String::with_capacity(32);
    for b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// PHP `var_export` for a string (single-quoted).
pub fn php_export_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            _ => out.push(ch),
        }
    }
    out.push('\'');
    out
}

/// Path expression relative to `vendor/composer/`, Composer style.
///
/// - `vendor/foo/bar/src` -> `$vendorDir . '/foo/bar/src'`
/// - `app` -> `$baseDir . '/app'`
/// - `.` -> `$baseDir . ''`  (Composer uses `$baseDir . ''` for empty remaining)
pub fn path_code(rel_from_project: &str) -> String {
    let rel = rel_from_project.replace('\\', "/");
    if let Some(rest) = rel.strip_prefix("vendor/") {
        return format!("$vendorDir . {}", php_export_string(&format!("/{rest}")));
    }
    if rel == "vendor" {
        return "$vendorDir . ''".to_owned();
    }
    // Project-root relative
    let rest = if rel == "." {
        String::new()
    } else {
        format!("/{rel}")
    };
    format!("$baseDir . {}", php_export_string(&rest))
}

/// Static-init path code relative to `vendor/composer/` (`__DIR__`).
///
/// - vendor paths: `__DIR__ . '/..' . '/foo/bar'`
/// - base paths: `__DIR__ . '/../..' . '/app'`
pub fn static_path_code(rel_from_project: &str) -> String {
    let rel = rel_from_project.replace('\\', "/");
    if let Some(rest) = rel.strip_prefix("vendor/") {
        return format!("__DIR__ . '/..' . {}", php_export_string(&format!("/{rest}")));
    }
    if rel == "vendor" {
        return "__DIR__ . '/..' . ''".to_owned();
    }
    let rest = if rel == "." {
        String::new()
    } else {
        format!("/{rel}")
    };
    format!("__DIR__ . '/../..' . {}", php_export_string(&rest))
}
