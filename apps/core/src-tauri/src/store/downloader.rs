//! Downloader: fetch a package archive + verify sha256.

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};

pub struct Downloader;

impl Downloader {
    pub fn download(url: &str, expected_sha256: Option<&str>) -> Result<Vec<u8>> {
        let resp = reqwest::blocking::get(url)
            .context("download package")?
            .error_for_status()
            .context("download http error")?;
        let bytes = resp.bytes().context("read package bytes")?.to_vec();

        if let Some(expected) = expected_sha256 {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let actual = hex::encode(hasher.finalize());
            if actual != expected {
                return Err(anyhow!(
                    "sha256 mismatch: expected {expected}, got {actual}"
                ));
            }
        }
        Ok(bytes)
    }
}
