use anyhow::{Result, bail};
use cmk_core::manifest::Manifest;
use serde::{Deserialize, Serialize};
use url::Url;

pub mod github;
pub mod http;
pub mod mirror;

pub use github::GithubReleases;
pub use mirror::HttpMirror;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    pub versions: Vec<String>,
}

/// Pluggable source of release manifests + tarballs (design §6.4).
///
/// Closed enum dispatch (instead of `dyn Registry`) — keeps the API
/// async-fn-in-trait friendly without `async-trait` and avoids vtable
/// indirection for what is realistically a small set of implementations.
pub enum RegistryClient {
    Github(GithubReleases),
    Mirror(HttpMirror),
}

impl RegistryClient {
    pub fn name(&self) -> &str {
        match self {
            Self::Github(_) => "github",
            Self::Mirror(_) => "http-mirror",
        }
    }

    pub async fn fetch_index(&self) -> Result<Index> {
        match self {
            Self::Github(r) => r.fetch_index().await,
            Self::Mirror(r) => r.fetch_index().await,
        }
    }

    pub async fn fetch_manifest(&self, version: &str) -> Result<Manifest> {
        match self {
            Self::Github(r) => r.fetch_manifest(version).await,
            Self::Mirror(r) => r.fetch_manifest(version).await,
        }
    }

    pub fn tarball_url(&self, version: &str, platform: &str, package: &str) -> Result<Url> {
        match self {
            Self::Github(r) => r.tarball_url(version, platform, package),
            Self::Mirror(r) => r.tarball_url(version, platform, package),
        }
    }
}

/// Parse a single `registries = [...]` entry into a concrete client.
///
/// Accepted forms:
/// - `github:<owner>/<repo>`
/// - `http://...`, `https://...` — interpreted as `HttpMirror`
pub fn parse_registry(spec: &str) -> Result<RegistryClient> {
    if let Some(rest) = spec.strip_prefix("github:") {
        if !rest.contains('/') {
            bail!("github registry must be `github:<owner>/<repo>`, got `{spec}`");
        }
        return Ok(RegistryClient::Github(GithubReleases::new(rest)));
    }
    if spec.starts_with("http://") || spec.starts_with("https://") {
        let base = Url::parse(spec)?;
        return Ok(RegistryClient::Mirror(HttpMirror::new(base)));
    }
    bail!("unrecognized registry spec: `{spec}`");
}

/// Walk every registry in order, returning the first manifest that
/// resolves cleanly. Errors from individual registries are collected
/// for diagnostics if all fail.
pub async fn fetch_manifest_any(specs: &[String], version: &str) -> Result<Manifest> {
    if specs.is_empty() {
        bail!(
            "no registries configured (add `registries = [...]` to ~/.config/cmk/config.toml \
             or use `cmk toolchain install --manifest <path>`)"
        );
    }
    let mut errs: Vec<String> = Vec::new();
    for s in specs {
        match parse_registry(s) {
            Ok(reg) => match reg.fetch_manifest(version).await {
                Ok(m) => return Ok(m),
                Err(e) => errs.push(format!("{s}: {e}")),
            },
            Err(e) => errs.push(format!("{s}: {e}")),
        }
    }
    bail!(
        "no registry could resolve version `{version}`:\n  {}",
        errs.join("\n  ")
    )
}

pub async fn fetch_index_first(specs: &[String]) -> Result<Index> {
    if specs.is_empty() {
        bail!("no registries configured");
    }
    let mut errs: Vec<String> = Vec::new();
    for s in specs {
        match parse_registry(s) {
            Ok(reg) => match reg.fetch_index().await {
                Ok(idx) => return Ok(idx),
                Err(e) => errs.push(format!("{s}: {e}")),
            },
            Err(e) => errs.push(format!("{s}: {e}")),
        }
    }
    bail!("no registry returned an index:\n  {}", errs.join("\n  "))
}
