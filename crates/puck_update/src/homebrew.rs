//! Detect Homebrew-managed puck binaries.

use std::path::Path;

/// True when `exe` looks like a Homebrew Cellar / prefix install of puck.
pub fn is_homebrew_managed(exe: &Path) -> bool {
    let lossy = exe.to_string_lossy();
    if lossy.contains("/Cellar/puck/") || lossy.contains("/Cellar/puck@") {
        return true;
    }

    let resolved = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    let resolved_s = resolved.to_string_lossy();
    if resolved_s.contains("/Cellar/puck/") || resolved_s.contains("/Cellar/puck@") {
        return true;
    }

    if let Ok(prefix) = std::env::var("HOMEBREW_PREFIX") {
        let cellar = Path::new(&prefix).join("Cellar").join("puck");
        if resolved.starts_with(&cellar) || exe.starts_with(&cellar) {
            return true;
        }
        // Common opt symlink: $HOMEBREW_PREFIX/opt/puck/bin/puck
        let opt = Path::new(&prefix).join("opt").join("puck");
        if resolved.starts_with(&opt) || exe.starts_with(&opt) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detects_cellar_path() {
        let p = PathBuf::from("/opt/homebrew/Cellar/puck/0.1.0/bin/puck");
        assert!(is_homebrew_managed(&p));
    }

    #[test]
    fn ignores_user_install() {
        let p = PathBuf::from("/Users/me/.puck/bin/puck");
        assert!(!is_homebrew_managed(&p));
    }
}
