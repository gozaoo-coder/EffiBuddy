//! Remote registry client. Fetches the package index JSON from a GitHub
//! raw URL (configurable). MVP uses a single hardcoded repo.

use crate::store::RemotePackage;
use anyhow::{Context, Result};

const DEFAULT_INDEX_URL: &str =
    "https://raw.githubusercontent.com/desktop-suite/registry/main/index.json";

pub struct RegistryClient {
    url: String,
}

impl RegistryClient {
    pub fn new() -> Self {
        Self {
            url: DEFAULT_INDEX_URL.to_string(),
        }
    }

    pub fn with_url(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }

    pub fn fetch_index(&self) -> Result<Vec<RemotePackage>> {
        let resp = reqwest::blocking::get(&self.url)
            .context("fetch registry index")?
            .error_for_status()
            .context("registry index http error")?;
        let pkgs: Vec<RemotePackage> = resp.json().context("parse registry index")?;
        Ok(pkgs)
    }
}
