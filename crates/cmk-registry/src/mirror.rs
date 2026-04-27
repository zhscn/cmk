use anyhow::{Context, Result};
use cmk_core::manifest::Manifest;
use url::Url;

use crate::{Index, Registry, http};

/// Plain HTTP mirror with the same layout as `GithubReleases`, just
/// rooted at an arbitrary base URL (`<base>/<tag>/<asset>`).
pub struct HttpMirror {
    pub base: Url,
}

impl HttpMirror {
    pub fn new(base: Url) -> Self {
        Self { base }
    }

    fn join(&self, parts: &[&str]) -> Result<String> {
        let mut s = self.base.to_string();
        if !s.ends_with('/') {
            s.push('/');
        }
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                s.push('/');
            }
            s.push_str(p);
        }
        Ok(s)
    }
}

impl Registry for HttpMirror {
    fn name(&self) -> &str {
        "http-mirror"
    }

    fn fetch_index(&self) -> Result<Index> {
        let url = self.join(&["index", "index.json"])?;
        let body = http::get_string(&url)?;
        let idx: Index =
            serde_json::from_str(&body).with_context(|| format!("parse index.json from {url}"))?;
        Ok(idx)
    }

    fn fetch_manifest(&self, version: &str) -> Result<Manifest> {
        let tag = format!("v{version}");
        let url = self.join(&[&tag, "manifest.toml"])?;
        let body = http::get_string(&url)?;
        Manifest::from_toml(&body)
            .with_context(|| format!("parse manifest.toml from {url}"))
    }

    fn tarball_url(&self, version: &str, platform: &str, package: &str) -> Result<Url> {
        let tag = format!("v{version}");
        let asset = format!("clang-{version}-{platform}-{package}.tar.zst");
        Ok(Url::parse(&self.join(&[&tag, &asset])?)?)
    }
}
