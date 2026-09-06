//! Composer-compatible `auth.json` loading for dist downloads.
//!
//! Merge order (later wins): `$COMPOSER_HOME/auth.json` (default `~/.composer/auth.json`),
//! project `./auth.json`, env `COMPOSER_AUTH` JSON.
//!
//! Supports `http-basic`, `bearer`, and `github-oauth` (github.com / api.github.com /
//! codeload.github.com). Secrets are never logged.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Authorization to attach to an HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthHeader {
    Basic { username: String, password: String },
    Bearer(String),
    /// GitHub token style: `Authorization: token …`
    GithubToken(String),
}

/// In-memory auth map keyed by host (lowercase).
#[derive(Clone, Default)]
pub struct AuthStore {
    http_basic: BTreeMap<String, (String, String)>,
    bearer: BTreeMap<String, String>,
    /// Token from `github-oauth` for `github.com` (Composer shape).
    github_oauth: Option<String>,
}

impl std::fmt::Debug for AuthStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthStore")
            .field("http_basic_hosts", &self.http_basic.keys().collect::<Vec<_>>())
            .field("bearer_hosts", &self.bearer.keys().collect::<Vec<_>>())
            .field("github_oauth", &self.github_oauth.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

impl AuthStore {
    /// Load auth with Composer merge order for the given locations / env JSON.
    ///
    /// Missing files are ignored. Invalid JSON in a file returns an error.
    /// `COMPOSER_AUTH` parse failures return an error when `env_json` is `Some`.
    pub fn load_merged(
        composer_home: Option<&Path>,
        project_dir: Option<&Path>,
        env_json: Option<&str>,
    ) -> crate::Result<Self> {
        let mut store = Self::default();
        if let Some(home) = composer_home {
            let path = home.join("auth.json");
            if path.is_file() {
                store.merge_file(&path)?;
            }
        }
        if let Some(project) = project_dir {
            let path = project.join("auth.json");
            if path.is_file() {
                store.merge_file(&path)?;
            }
        }
        if let Some(raw) = env_json {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                let value: Value = serde_json::from_str(trimmed).map_err(|e| {
                    crate::Error::Auth(format!("COMPOSER_AUTH is not valid JSON: {e}"))
                })?;
                store.merge_value(&value);
            }
        }
        Ok(store)
    }

    /// Load using `$COMPOSER_HOME` / `~/.composer`, `current_dir`, and `$COMPOSER_AUTH`.
    pub fn load_from_env() -> crate::Result<Self> {
        let home = composer_home_dir();
        let project = std::env::current_dir().ok();
        let env_json = std::env::var("COMPOSER_AUTH").ok();
        Self::load_merged(
            home.as_deref(),
            project.as_deref(),
            env_json.as_deref(),
        )
    }

    fn merge_file(&mut self, path: &Path) -> crate::Result<()> {
        let bytes = std::fs::read(path).map_err(|e| {
            crate::Error::Auth(format!("read {}: {e}", path.display()))
        })?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|e| {
            crate::Error::Auth(format!("invalid {}: {e}", path.display()))
        })?;
        self.merge_value(&value);
        Ok(())
    }

    /// Merge one auth.json object into this store (later entries win per host).
    pub fn merge_value(&mut self, value: &Value) {
        let Some(obj) = value.as_object() else {
            return;
        };

        if let Some(Value::Object(map)) = obj.get("http-basic") {
            for (host, creds) in map {
                let Some(c) = creds.as_object() else {
                    continue;
                };
                let username = c
                    .get("username")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let password = c
                    .get("password")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                self.http_basic
                    .insert(normalize_host(host), (username, password));
            }
        }

        if let Some(Value::Object(map)) = obj.get("bearer") {
            for (host, token) in map {
                if let Some(t) = token.as_str() {
                    self.bearer.insert(normalize_host(host), t.to_owned());
                }
            }
        }

        if let Some(Value::Object(map)) = obj.get("github-oauth") {
            // Composer keys by domain; we honour github.com primarily.
            if let Some(token) = map
                .get("github.com")
                .and_then(|v| v.as_str())
                .or_else(|| map.values().find_map(|v| v.as_str()))
            {
                self.github_oauth = Some(token.to_owned());
            }
        }
    }

    /// Pick an Authorization header for `url` based on its host.
    pub fn authorization_for_url(&self, url: &str) -> Option<AuthHeader> {
        let host = host_from_url(url)?;
        let host_key = normalize_host(&host);

        if is_github_download_host(&host_key) {
            if let Some(token) = &self.github_oauth {
                return Some(AuthHeader::GithubToken(token.clone()));
            }
        }

        if let Some(token) = self.bearer.get(&host_key) {
            return Some(AuthHeader::Bearer(token.clone()));
        }

        if let Some((username, password)) = self.http_basic.get(&host_key) {
            return Some(AuthHeader::Basic {
                username: username.clone(),
                password: password.clone(),
            });
        }

        None
    }
}

/// Resolve `$COMPOSER_HOME`, defaulting to `~/.composer`.
pub fn composer_home_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("COMPOSER_HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".composer"))
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn host_from_url(url: &str) -> Option<String> {
    // Minimal parse: scheme://host[:port]/path
    let rest = url.split("://").nth(1)?;
    let host_port = rest.split('/').next()?.split('?').next()?;
    let host = host_port.split('@').next_back()?; // strip userinfo if present
    let host = host.split(':').next()?.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_owned())
    }
}

fn is_github_download_host(host: &str) -> bool {
    matches!(
        host,
        "github.com" | "api.github.com" | "codeload.github.com"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "puck-auth-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn merge_order_later_wins_http_basic() {
        let home = temp_dir();
        let project = temp_dir();
        fs::write(
            home.join("auth.json"),
            r#"{"http-basic":{"repo.example.com":{"username":"home-user","password":"home-pass"}}}"#,
        )
        .unwrap();
        fs::write(
            project.join("auth.json"),
            r#"{"http-basic":{"repo.example.com":{"username":"proj-user","password":"proj-pass"}}}"#,
        )
        .unwrap();

        let store = AuthStore::load_merged(Some(&home), Some(&project), None).unwrap();
        assert_eq!(
            store.authorization_for_url("https://repo.example.com/pkg.zip"),
            Some(AuthHeader::Basic {
                username: "proj-user".into(),
                password: "proj-pass".into(),
            })
        );

        let store_env = AuthStore::load_merged(
            Some(&home),
            Some(&project),
            Some(r#"{"http-basic":{"repo.example.com":{"username":"env-user","password":"env-pass"}}}"#),
        )
        .unwrap();
        assert_eq!(
            store_env.authorization_for_url("https://repo.example.com/pkg.zip"),
            Some(AuthHeader::Basic {
                username: "env-user".into(),
                password: "env-pass".into(),
            })
        );

        let _ = fs::remove_dir_all(&home);
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn bearer_and_github_oauth_by_host() {
        let project = temp_dir();
        fs::write(
            project.join("auth.json"),
            r#"{
                "bearer": { "packs.example.com": "pack-token" },
                "github-oauth": { "github.com": "gh-secret" }
            }"#,
        )
        .unwrap();
        let store = AuthStore::load_merged(None, Some(&project), None).unwrap();

        assert_eq!(
            store.authorization_for_url("https://packs.example.com/a.tgz"),
            Some(AuthHeader::Bearer("pack-token".into()))
        );
        assert_eq!(
            store.authorization_for_url("https://api.github.com/repos/o/r/zipball/abc"),
            Some(AuthHeader::GithubToken("gh-secret".into()))
        );
        assert_eq!(
            store.authorization_for_url("https://codeload.github.com/o/r/legacy.zip/abc"),
            Some(AuthHeader::GithubToken("gh-secret".into()))
        );
        assert!(store
            .authorization_for_url("https://other.example.com/x.zip")
            .is_none());

        // Debug formatting must not embed the secret token value for AuthStore.
        let dbg = format!("{store:?}");
        assert!(
            !dbg.contains("gh-secret"),
            "AuthStore Debug must not include oauth token"
        );

        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn host_matching_is_case_insensitive() {
        let mut store = AuthStore::default();
        store.merge_value(&serde_json::json!({
            "http-basic": {
                "Repo.Example.COM": { "username": "u", "password": "p" }
            }
        }));
        assert_eq!(
            store.authorization_for_url("https://repo.example.com/f.zip"),
            Some(AuthHeader::Basic {
                username: "u".into(),
                password: "p".into(),
            })
        );
    }
}
