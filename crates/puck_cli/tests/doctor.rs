use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn puck_bin() -> &'static str {
    env!("CARGO_BIN_EXE_puck")
}

fn fixture(name: &str) -> std::path::PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

use std::path::PathBuf;

#[test]
fn doctor_laravel_skeleton_ready() {
    let out = Command::new(puck_bin())
        .args(["doctor", "--working-dir"])
        .arg(fixture("laravel-skeleton"))
        .output()
        .expect("run doctor");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.lines().last() == Some("ready to switch"),
        "stdout={stdout}"
    );
}

#[test]
fn doctor_laravel_app_ready() {
    let out = Command::new(puck_bin())
        .args(["doctor", "--working-dir"])
        .arg(fixture("laravel-app"))
        .output()
        .expect("run doctor");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.lines().last() == Some("ready to switch"),
        "stdout={stdout}"
    );
}

#[test]
fn doctor_unsupported_plugin_exits_1() {
    let dir = tempdir().unwrap();
    let json = r#"{
      "name": "acme/tmp",
      "require": {},
      "config": { "allow-plugins": { "weird/plugin": true } }
    }"#;
    let hash = puck_lock::content_hash(json).unwrap();
    let lock = format!(
        r#"{{
      "content-hash": "{hash}",
      "packages": [{{
        "name": "weird/plugin",
        "version": "1.0.0",
        "type": "composer-plugin"
      }}],
      "packages-dev": [],
      "aliases": [],
      "minimum-stability": "stable",
      "stability-flags": {{}},
      "prefer-stable": true,
      "prefer-lowest": false,
      "platform": {{}},
      "platform-dev": {{}},
      "plugin-api-version": "2.9.0"
    }}"#
    );
    fs::write(dir.path().join("composer.json"), json).unwrap();
    fs::write(dir.path().join("composer.lock"), lock).unwrap();
    let out = Command::new(puck_bin())
        .args(["doctor", "--working-dir"])
        .arg(dir.path())
        .output()
        .expect("run doctor");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "stdout={stdout}");
    assert!(stdout.contains("weird/plugin"));
    assert!(stdout.contains("not planned"));
    assert!(stdout.lines().last().unwrap().contains("blocker"));
}

#[test]
fn doctor_vcs_repo_ready() {
    let dir = tempdir().unwrap();
    let json = r#"{
      "name": "acme/tmp",
      "require": {},
      "repositories": [ { "type": "vcs", "url": "https://github.com/acme/foo" } ]
    }"#;
    let hash = puck_lock::content_hash(json).unwrap();
    let lock = format!(
        r#"{{
      "content-hash": "{hash}",
      "packages": [],
      "packages-dev": [],
      "aliases": [],
      "minimum-stability": "stable",
      "stability-flags": {{}},
      "prefer-stable": true,
      "prefer-lowest": false,
      "platform": {{}},
      "platform-dev": {{}},
      "plugin-api-version": "2.9.0"
    }}"#
    );
    fs::write(dir.path().join("composer.json"), json).unwrap();
    fs::write(dir.path().join("composer.lock"), lock).unwrap();
    let out = Command::new(puck_bin())
        .args(["doctor", "--working-dir"])
        .arg(dir.path())
        .output()
        .expect("run doctor");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stdout={stdout}");
    assert!(stdout.contains("ready to switch"), "stdout={stdout}");
}
