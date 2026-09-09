//! Git CLI driver (Composer `GitDriver` subset).

use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One version discovered in a git VCS repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitPackageVersion {
    /// Composer pretty version (`1.2.3`, `dev-main`, …).
    pub pretty_version: String,
    /// Git ref name (tag or branch) used with `git show` / checkout.
    pub ref_name: String,
    /// Resolved commit object name (sha).
    pub reference: String,
    /// True when this version came from a branch (`dev-*`).
    pub is_branch: bool,
}

/// Default VCS mirror cache: `~/.puck/cache/vcs`.
pub fn default_vcs_cache_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".puck").join("cache").join("vcs")
}

/// Stable cache directory for a repository URL under `cache_root`.
pub fn mirror_dir_for(cache_root: &Path, url: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let hex = hex_encode(&hasher.finalize());
    cache_root.join(&hex[..16.min(hex.len())])
}

/// Clone or update a bare mirror of `url` under `cache_root`. Returns mirror path.
pub fn ensure_git_mirror(cache_root: &Path, url: &str) -> Result<PathBuf> {
    need_git()?;
    let mirror = mirror_dir_for(cache_root, url);
    if mirror.join("HEAD").is_file() || mirror.join("refs").is_dir() {
        git_ok(&[
            "-C",
            &mirror.to_string_lossy(),
            "remote",
            "update",
            "--prune",
        ])?;
        return Ok(mirror);
    }
    if mirror.exists() {
        fs::remove_dir_all(&mirror).map_err(|e| {
            Error::Message(format!(
                "remove incomplete vcs cache {}: {e}",
                mirror.display()
            ))
        })?;
    }
    if let Some(parent) = mirror.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| Error::Message(format!("create vcs cache {}: {e}", parent.display())))?;
    }
    // Local path repos: allow bare or worktree URLs without file://.
    let clone_url = normalize_clone_url(url);
    git_ok(&["clone", "--mirror", &clone_url, &mirror.to_string_lossy()])?;
    Ok(mirror)
}

/// List tag and branch versions from an existing mirror.
pub fn list_git_versions(mirror: &Path) -> Result<Vec<GitPackageVersion>> {
    need_git()?;
    let mut out = Vec::new();

    let tags = git_stdout(&["-C", &mirror.to_string_lossy(), "tag", "-l"])?;
    for tag in tags.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let pretty = pretty_from_tag(tag);
        let reference = rev_parse(mirror, tag)?;
        out.push(GitPackageVersion {
            pretty_version: pretty,
            ref_name: tag.to_string(),
            reference,
            is_branch: false,
        });
    }

    let heads = git_stdout(&[
        "-C",
        &mirror.to_string_lossy(),
        "for-each-ref",
        "--format=%(refname:short)",
        "refs/heads",
    ])?;
    for branch in heads.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let pretty = format!("dev-{branch}");
        let reference = rev_parse(mirror, branch)?;
        out.push(GitPackageVersion {
            pretty_version: pretty,
            ref_name: branch.to_string(),
            reference,
            is_branch: true,
        });
    }

    Ok(out)
}

/// Read `composer.json` blob at `ref_name` from the mirror (`git show ref:composer.json`).
pub fn read_composer_json_at(mirror: &Path, ref_name: &str) -> Result<String> {
    need_git()?;
    let spec = format!("{ref_name}:composer.json");
    git_stdout(&["-C", &mirror.to_string_lossy(), "show", &spec])
}

/// Checkout `reference` (commit / tag / branch) from `mirror` into `dest` (new directory).
pub fn checkout_reference(mirror: &Path, reference: &str, dest: &Path) -> Result<()> {
    need_git()?;
    if dest.exists() {
        return Err(Error::Message(format!(
            "checkout dest already exists: {}",
            dest.display()
        )));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| Error::Message(format!("create {}: {e}", parent.display())))?;
    }
    git_ok(&[
        "clone",
        "--no-checkout",
        &mirror.to_string_lossy(),
        &dest.to_string_lossy(),
    ])?;
    git_ok(&[
        "-C",
        &dest.to_string_lossy(),
        "checkout",
        "--force",
        reference,
    ])?;
    // Drop .git so vendor trees are not nested repos (Composer keeps .git for source
    // installs; puck links from store-style trees - keep .git for authenticity of source).
    Ok(())
}

fn pretty_from_tag(tag: &str) -> String {
    let t = tag.trim();
    if let Some(rest) = t.strip_prefix('v').or_else(|| t.strip_prefix('V'))
        && !rest.is_empty()
        && rest.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        return rest.to_string();
    }
    t.to_string()
}

fn rev_parse(mirror: &Path, rev: &str) -> Result<String> {
    let out = git_stdout(&["-C", &mirror.to_string_lossy(), "rev-parse", rev])?;
    let sha = out.trim();
    if sha.is_empty() {
        return Err(Error::Git(format!("empty rev-parse for {rev}")));
    }
    Ok(sha.to_string())
}

fn normalize_clone_url(url: &str) -> String {
    let u = url.trim();
    if u.starts_with("git@")
        || u.starts_with("ssh://")
        || u.starts_with("http://")
        || u.starts_with("https://")
        || u.starts_with("file://")
        || u.starts_with("git://")
    {
        return u.to_string();
    }
    // Absolute or relative local path.
    Path::new(u)
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| u.to_string())
}

fn need_git() -> Result<()> {
    match Command::new("git").arg("--version").output() {
        Ok(o) if o.status.success() => Ok(()),
        _ => Err(Error::Message(
            "git is required for vcs repositories (not found on PATH)".into(),
        )),
    }
}

fn git_ok(args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| Error::Git(format!("spawn git: {e}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(Error::Git(format_git_failure(args, &output)))
}

fn git_stdout(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| Error::Git(format!("spawn git: {e}")))?;
    if !output.status.success() {
        return Err(Error::Git(format_git_failure(args, &output)));
    }
    String::from_utf8(output.stdout).map_err(|e| Error::Git(format!("git stdout utf8: {e}")))
}

fn format_git_failure(args: &[&str], output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    format!(
        "git {} failed ({}): {}{}",
        args.join(" "),
        output.status,
        stderr.trim(),
        if stdout.trim().is_empty() {
            String::new()
        } else {
            format!(" / {}", stdout.trim())
        }
    )
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn init_repo(dir: &Path) -> String {
        fs::create_dir_all(dir).unwrap();
        run(
            dir,
            &["git", "-c", "init.templateDir=", "init", "-b", "main"],
        );
        run(dir, &["git", "config", "user.email", "puck@test"]);
        run(dir, &["git", "config", "user.name", "puck"]);
        fs::write(
            dir.join("composer.json"),
            r#"{"name":"acme/vcs-hello","description":"fixture"}"#,
        )
        .unwrap();
        run(dir, &["git", "add", "composer.json"]);
        run(dir, &["git", "commit", "-m", "init"]);
        run(dir, &["git", "tag", "v1.0.0"]);
        fs::write(
            dir.join("composer.json"),
            r#"{"name":"acme/vcs-hello","description":"fixture","version":"ignored"}"#,
        )
        .unwrap();
        run(dir, &["git", "add", "composer.json"]);
        run(dir, &["git", "commit", "-m", "second"]);
        dir.canonicalize().unwrap().to_string_lossy().into_owned()
    }

    fn run(dir: &Path, cmd: &[&str]) {
        let st = Command::new(cmd[0])
            .args(&cmd[1..])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(st.success(), "{cmd:?}");
    }

    #[test]
    fn mirror_lists_tag_and_branch_and_reads_composer() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let url = init_repo(&repo);
        let cache = tmp.path().join("cache");
        let mirror = ensure_git_mirror(&cache, &url).expect("mirror");
        let versions = list_git_versions(&mirror).expect("list");
        assert!(
            versions
                .iter()
                .any(|v| v.pretty_version == "1.0.0" && !v.is_branch),
            "{versions:?}"
        );
        assert!(
            versions
                .iter()
                .any(|v| v.pretty_version == "dev-main" && v.is_branch),
            "{versions:?}"
        );
        let body = read_composer_json_at(&mirror, "v1.0.0").expect("show");
        assert!(body.contains("acme/vcs-hello"));
    }

    #[test]
    fn checkout_reference_writes_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let url = init_repo(&repo);
        let cache = tmp.path().join("cache");
        let mirror = ensure_git_mirror(&cache, &url).unwrap();
        let versions = list_git_versions(&mirror).unwrap();
        let tag = versions
            .iter()
            .find(|v| v.pretty_version == "1.0.0")
            .unwrap();
        let dest = tmp.path().join("out");
        checkout_reference(&mirror, &tag.reference, &dest).unwrap();
        assert!(dest.join("composer.json").is_file());
    }
}
