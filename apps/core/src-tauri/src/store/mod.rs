//! Package store backend: remote index client, downloader, extractor.

pub mod downloader;
pub mod extractor;
pub mod registry_client;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePackage {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub download_url: String,
    pub sha256: Option<String>,
    pub size: u64,
}
