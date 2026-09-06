//! HTTP helpers for manifest / artifact download.

use crate::error::{Error, Result};
use crate::manifest::Manifest;
use std::time::Duration;

pub fn user_agent(version: &str, target: &str) -> String {
    format!("puck/{version} ({target})")
}

pub fn http_client(timeout: Duration, version: &str, target: &str) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(user_agent(version, target))
        .build()
        .map_err(|e| Error::Message(format!("http client: {e}")))
}

pub fn fetch_bytes(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| Error::Download {
            url: url.to_owned(),
            message: e.to_string(),
        })?;
    if !resp.status().is_success() {
        return Err(Error::Download {
            url: url.to_owned(),
            message: format!("HTTP {}", resp.status()),
        });
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| Error::Download {
            url: url.to_owned(),
            message: e.to_string(),
        })
}

pub fn fetch_manifest(client: &reqwest::blocking::Client, url: &str) -> Result<Manifest> {
    let bytes = fetch_bytes(client, url)?;
    Manifest::parse(&bytes)
}
