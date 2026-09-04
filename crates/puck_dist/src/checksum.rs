//! Checksums. Composer dist `shasum` is SHA-1.

use crate::{Error, Result};
use sha1::{Digest as _, Sha1};
use sha2::Sha256;

pub fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

/// Verify Composer `dist.shasum` when present (SHA-1 hex).
pub fn verify_shasum(bytes: &[u8], expected: &str, url: &str) -> Result<()> {
    if expected.is_empty() {
        return Ok(());
    }
    let actual = sha1_hex(bytes);
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(Error::ChecksumMismatch {
            url: url.to_owned(),
            expected: expected.to_owned(),
            actual,
        })
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_empty() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn verify_accepts_empty_expected() {
        verify_shasum(b"x", "", "http://example").expect("ok");
    }
}
