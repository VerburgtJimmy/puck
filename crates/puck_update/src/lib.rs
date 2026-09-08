//! Self-upgrade, rollback, and update notifications for the puck CLI.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

mod error;
mod fetch;
mod homebrew;
mod manifest;
mod notify;
mod replace;
mod target;
mod upgrade;
mod urls;
mod verify;

pub use error::{Error, Result};
pub use homebrew::is_homebrew_managed;
pub use manifest::{Artifact, Manifest};
pub use notify::{NotifyOptions, maybe_notify_update};
pub use replace::{PREVIOUS_NAME, default_bin_dir, default_cache_dir, default_puck_root};
pub use target::detect_target;
pub use upgrade::{UpgradeOptions, UpgradeOutcome, rollback_exe, upgrade};
pub use urls::{
    DEFAULT_INSTALL_ROOT_NAME, INSTALL_MIRROR, MANIFEST_URL, MINISIGN_PUBLIC_KEY, REPO,
    STABLE_MIRROR, manifest_url_for_version,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::sha256_hex;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;
    use tar::Builder;

    fn make_tarball(bin_contents: &[u8]) -> Vec<u8> {
        let mut raw = Vec::new();
        {
            let enc = GzEncoder::new(&mut raw, Compression::fast());
            let mut builder = Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            header.set_size(bin_contents.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "puck", bin_contents)
                .expect("append");
            builder.into_inner().expect("finish").finish().expect("gz");
        }
        raw
    }

    #[test]
    fn upgrade_from_fake_manifest() {
        let payload = b"fake-puck-binary-v2";
        let tarball = make_tarball(payload);
        let sha = sha256_hex(&tarball);
        let target = detect_target().expect("target");

        let dir = tempfile::tempdir().expect("tmpdir");
        let current = dir.path().join("puck");
        std::fs::write(&current, b"fake-puck-binary-v1").expect("write current");

        // Build routes after we know base - use placeholder then rebuild.
        // Two-step: bind once with closure that reads shared state.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let base = format!("http://127.0.0.1:{port}");
        let archive_path = format!("/puck-{target}.tar.gz");
        let manifest = serde_json::json!({
            "version": "0.1.1",
            "released_at": "2026-09-06T00:00:00Z",
            "channel": "stable",
            "artifacts": {
                target: {
                    "url": format!("{base}{archive_path}"),
                    "sha256": sha,
                    "size": tarball.len()
                }
            }
        });
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("json");
        let sums = format!("{sha}  puck-{target}.tar.gz\n");
        let routes = Arc::new(vec![
            (
                "/manifest.json".to_string(),
                manifest_bytes,
                "application/json",
            ),
            (archive_path.clone(), tarball.clone(), "application/gzip"),
            ("/SHA256SUMS".to_string(), sums.into_bytes(), "text/plain"),
        ]);

        let routes_clone = Arc::clone(&routes);
        let _server = thread::spawn(move || {
            for stream in listener.incoming().take(16) {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf);
                let req = String::from_utf8_lossy(&buf);
                let path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/");
                let mut matched = None;
                for (p, body, ctype) in routes_clone.iter() {
                    if p == path {
                        matched = Some((body.clone(), *ctype));
                        break;
                    }
                }
                let (body, ctype) =
                    matched.unwrap_or_else(|| (b"not found".to_vec(), "text/plain"));
                let status = if body.as_slice() == b"not found" {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
            }
        });

        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).expect("work");
        let outcome = upgrade(UpgradeOptions {
            version: None,
            manifest_url: Some(format!("{base}/manifest.json")),
            current_version: "0.1.0".into(),
            current_exe: Some(current.clone()),
            work_dir: Some(work),
        })
        .expect("upgrade");

        assert_eq!(outcome.to_version, "0.1.1");
        assert_eq!(std::fs::read(&current).expect("read"), payload);
        assert_eq!(
            std::fs::read(dir.path().join(PREVIOUS_NAME)).expect("previous"),
            b"fake-puck-binary-v1"
        );
        // minisign absent → skipped warning expected
        assert!(!outcome.minisign_verified);
    }

    #[test]
    fn homebrew_refuses_upgrade() {
        let dir = tempfile::tempdir().expect("tmpdir");
        // Path contains Cellar/puck
        let cellar = dir.path().join("Cellar/puck/0.1.0/bin");
        std::fs::create_dir_all(&cellar).expect("mkdir");
        let current = cellar.join("puck");
        std::fs::write(&current, b"brew").expect("write");

        let err = upgrade(UpgradeOptions {
            version: None,
            manifest_url: Some("http://127.0.0.1:1/manifest.json".into()),
            current_version: "0.1.0".into(),
            current_exe: Some(current),
            work_dir: Some(dir.path().join("work")),
        })
        .unwrap_err();
        assert!(matches!(err, Error::HomebrewManaged(_)));
    }

    #[test]
    fn notify_respects_cache_interval() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let cache = dir.path().join("latest");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(
            &cache,
            serde_json::json!({ "checked_at": now, "latest_version": "0.1.0" }).to_string(),
        )
        .unwrap();

        // Should no-op without hitting network (invalid URL would fail if fetched).
        maybe_notify_update(NotifyOptions {
            current_version: "0.1.0".into(),
            manifest_url: Some("http://127.0.0.1:1/nope".into()),
            cache_path: Some(cache),
            force_tty: Some(true),
            ignore_env_disable: true,
        });
    }

    #[test]
    fn manifest_parse_roundtrip() {
        let raw = br#"{
          "version": "0.1.0",
          "channel": "stable",
          "artifacts": {
            "aarch64-apple-darwin": {
              "url": "https://example/puck.tar.gz",
              "sha256": "abc",
              "size": 1
            }
          }
        }"#;
        let m = Manifest::parse(raw).expect("parse");
        assert_eq!(m.version, "0.1.0");
        assert_eq!(
            m.artifact_for("aarch64-apple-darwin").unwrap().sha256,
            "abc"
        );
    }
}
