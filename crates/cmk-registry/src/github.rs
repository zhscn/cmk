use anyhow::{Context, Result};
use cmk_core::manifest::Manifest;
use url::Url;

use crate::{Index, http};

/// GitHub Releases-backed registry.
///
/// Layout (design §6.4):
/// - index:    `https://github.com/<repo>/releases/download/index/index.json`
/// - manifest: `https://github.com/<repo>/releases/download/v<ver>/manifest.toml`
/// - tarball:  `https://github.com/<repo>/releases/download/v<ver>/<asset>`
pub struct GithubReleases {
    pub repo: String,
}

impl GithubReleases {
    pub fn new(repo: impl Into<String>) -> Self {
        Self { repo: repo.into() }
    }

    fn asset_url(&self, tag: &str, asset: &str) -> String {
        format!(
            "https://github.com/{repo}/releases/download/{tag}/{asset}",
            repo = self.repo,
        )
    }

    pub async fn fetch_index(&self) -> Result<Index> {
        let url = self.asset_url("index", "index.json");
        let body = http::get_string(&url).await?;
        let idx: Index =
            serde_json::from_str(&body).with_context(|| format!("parse index.json from {url}"))?;
        Ok(idx)
    }

    pub async fn fetch_manifest(&self, version: &str) -> Result<Manifest> {
        let tag = format!("v{version}");
        let url = self.asset_url(&tag, "manifest.toml");
        let body = http::get_string(&url).await?;
        Manifest::from_toml(&body).with_context(|| format!("parse manifest.toml from {url}"))
    }

    pub fn tarball_url(&self, version: &str, platform: &str, package: &str) -> Result<Url> {
        let tag = format!("v{version}");
        let asset = format!("clang-{version}-{platform}-{package}.tar.zst");
        Ok(Url::parse(&self.asset_url(&tag, &asset))?)
    }
}
