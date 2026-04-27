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

/// Pluggable source of release manifests + tarballs (design §5.2).
pub trait Registry {
    fn name(&self) -> &str;
    fn fetch_index(&self) -> Result<Index>;
    fn fetch_manifest(&self, version: &str) -> Result<Manifest>;
    fn tarball_url(&self, version: &str, platform: &str, package: &str) -> Result<Url>;
}

/// Parse a single `registries = [...]` entry into a concrete client.
///
/// Accepted forms:
/// - `github:<owner>/<repo>`
/// - `http://...`, `https://...` — interpreted as `HttpMirror`
pub fn parse_registry(spec: &str) -> Result<Box<dyn Registry>> {
    if let Some(rest) = spec.strip_prefix("github:") {
        if !rest.contains('/') {
            bail!("github registry must be `github:<owner>/<repo>`, got `{spec}`");
        }
        return Ok(Box::new(GithubReleases::new(rest)));
    }
    if spec.starts_with("http://") || spec.starts_with("https://") {
        let base = Url::parse(spec)?;
        return Ok(Box::new(HttpMirror::new(base)));
    }
    bail!("unrecognized registry spec: `{spec}`");
}

/// Walk every registry in order, returning the first manifest that
/// resolves cleanly. Errors from individual registries are collected
/// for diagnostics if all fail.
pub fn fetch_manifest_any(specs: &[String], version: &str) -> Result<Manifest> {
    if specs.is_empty() {
        bail!(
            "no registries configured (add `registries = [...]` to ~/.cmk/config.toml \
             or use `cmk toolchain install --manifest <path>`)"
        );
    }
    let mut errs: Vec<String> = Vec::new();
    for s in specs {
        match parse_registry(s) {
            Ok(reg) => match reg.fetch_manifest(version) {
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

pub fn fetch_index_first(specs: &[String]) -> Result<Index> {
    if specs.is_empty() {
        bail!("no registries configured");
    }
    let mut errs: Vec<String> = Vec::new();
    for s in specs {
        match parse_registry(s).and_then(|r| r.fetch_index()) {
            Ok(idx) => return Ok(idx),
            Err(e) => errs.push(format!("{s}: {e}")),
        }
    }
    bail!("no registry returned an index:\n  {}", errs.join("\n  "))
}
