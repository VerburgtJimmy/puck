//! HTTP download of dist archives.

use crate::checksum::{sha256_hex, verify_shasum};
use crate::{Error, Result};
use std::time::Duration;

/// Bytes downloaded for a dist URL, with content hash for the store.
#[derive(Debug, Clone)]
pub struct DownloadedDist {
    pub url: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// Download `url` and optionally verify Composer `shasum` (SHA-1).
pub async fn download(url: &str, shasum: Option<&str>) -> Result<DownloadedDist> {
    let candidates = download_candidates(url);
    let mut last_err = None;
    for candidate in &candidates {
        match download_with_retries(candidate, 3).await {
            Ok(bytes) => {
                if let Some(sum) = shasum {
                    verify_shasum(&bytes, sum, candidate)?;
                }
                let sha256 = sha256_hex(&bytes);
                return Ok(DownloadedDist {
                    url: candidate.clone(),
                    bytes,
                    sha256,
                });
            }
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.unwrap_or_else(|| Error::Download {
        url: url.to_owned(),
        message: "no download candidates".into(),
    }))
}

fn download_candidates(url: &str) -> Vec<String> {
    let mut out = vec![url.to_owned()];
    if let Some(alt) = github_api_to_codeload(url) {
        out.push(alt);
    }
    out
}

/// Map `https://api.github.com/repos/{o}/{r}/zipball/{ref}` to codeload,
/// which is less aggressive about anonymous rate limits.
fn github_api_to_codeload(url: &str) -> Option<String> {
    const PREFIX: &str = "https://api.github.com/repos/";
    const MID: &str = "/zipball/";
    if !url.starts_with(PREFIX) {
        return None;
    }
    let rest = &url[PREFIX.len()..];
    let (repo, reference) = rest.split_once(MID)?;
    if repo.is_empty() || reference.is_empty() {
        return None;
    }
    Some(format!(
        "https://codeload.github.com/{repo}/legacy.zip/{reference}"
    ))
}

async fn download_with_retries(url: &str, attempts: u32) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("puck/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| Error::Download {
            url: url.to_owned(),
            message: e.to_string(),
        })?;

    let mut last_message = String::new();
    for attempt in 1..=attempts {
        match download_once(&client, url).await {
            Ok(bytes) => return Ok(bytes),
            Err(err) => {
                last_message = err.to_string();
                if attempt < attempts {
                    let backoff = Duration::from_millis(200 * u64::from(attempt));
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    Err(Error::Download {
        url: url.to_owned(),
        message: last_message,
    })
}

async fn download_once(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = client.get(url).send().await.map_err(|e| Error::Download {
        url: url.to_owned(),
        message: e.to_string(),
    })?;

    if !response.status().is_success() {
        return Err(Error::Download {
            url: url.to_owned(),
            message: format!("HTTP {}", response.status()),
        });
    }

    let bytes = response.bytes().await.map_err(|e| Error::Download {
        url: url.to_owned(),
        message: e.to_string(),
    })?;
    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_github_api_zipball() {
        let url = "https://api.github.com/repos/doctrine/lexer/zipball/31ad66abc0fc9e1a1f2d9bc6a42668d2fbbcd6dd";
        assert_eq!(
            github_api_to_codeload(url).as_deref(),
            Some(
                "https://codeload.github.com/doctrine/lexer/legacy.zip/31ad66abc0fc9e1a1f2d9bc6a42668d2fbbcd6dd"
            )
        );
    }
}
