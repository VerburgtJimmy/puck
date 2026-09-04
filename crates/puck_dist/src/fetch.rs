//! HTTP download of dist archives.

use crate::checksum::{sha256_hex, verify_shasum};
use crate::{Error, Result};

/// Bytes downloaded for a dist URL, with content hash for the store.
#[derive(Debug, Clone)]
pub struct DownloadedDist {
    pub url: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// Download `url` and optionally verify Composer `shasum` (SHA-1).
pub async fn download(url: &str, shasum: Option<&str>) -> Result<DownloadedDist> {
    let response = reqwest::Client::new()
        .get(url)
        .header(
            reqwest::header::USER_AGENT,
            concat!("puck/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await
        .map_err(|e| Error::Download {
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
    let bytes = bytes.to_vec();

    if let Some(sum) = shasum {
        verify_shasum(&bytes, sum, url)?;
    }

    let sha256 = sha256_hex(&bytes);
    Ok(DownloadedDist {
        url: url.to_owned(),
        bytes,
        sha256,
    })
}
