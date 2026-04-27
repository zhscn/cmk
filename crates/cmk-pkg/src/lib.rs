use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::{
    collections::HashMap,
    fmt::{self, Display},
    path::PathBuf,
};
use tokio::task::JoinHandle;

/// `$XDG_CONFIG_HOME/cmk/` (typically `~/.config/cmk/`).
pub fn config_dir() -> Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| anyhow!("cannot resolve $XDG_CONFIG_HOME / $HOME"))?;
    Ok(base.join("cmk"))
}

/// Canonical location of the global package index per design.md §3.1.
pub fn pkg_index_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("pkg.json"))
}

/// Cached metadata for the bundled CPM bootstrap script.
pub fn cpm_info_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("cpm.json"))
}

#[derive(Debug, Serialize, Deserialize, Hash, Clone)]
pub struct Package {
    pub owner: String,
    pub repo: String,
}

impl Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageIndex {
    pub aliases: HashMap<String, Package>,
    pub releases: HashMap<String, String>,
}

impl PackageIndex {
    pub fn load_or_create(path: &PathBuf) -> Result<Self> {
        if !path.try_exists()? {
            let index = Self {
                aliases: HashMap::new(),
                releases: HashMap::new(),
            };
            index.save(path)?;
            return Ok(index);
        }
        let content = std::fs::read_to_string(path)?;
        let index: PackageIndex = serde_json::from_str(&content)?;
        Ok(index)
    }

    pub fn save(&self, path: &PathBuf) -> Result<()> {
        let content = serde_json::to_string(self)?;
        let parent = path
            .parent()
            .with_context(|| "Failed to get parent directory")?;
        if !parent.try_exists()? {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn get_pkg_name(&self, name: &str) -> Result<String> {
        let pkg_name = if name.contains('/') {
            Some(name.to_string())
        } else {
            self.aliases.get(name).map(|s| s.to_string())
        };
        let pkg_name = pkg_name.with_context(|| format!("Package alias {name} not found"))?;
        Ok(pkg_name)
    }

    pub fn get_release(&self, name: &str) -> Result<&str> {
        let name = self.get_pkg_name(name)?;
        let release = self.releases.get(&name).map(|s| s.as_str());
        release.with_context(|| format!("Release {name} not found"))
    }

    pub async fn add_repo(&mut self, owner: &str, repo: &str) -> Result<()> {
        let octocrab = octocrab::instance();
        let release = octocrab.repos(owner, repo).releases().get_latest().await?;
        let pkg_name = format!("{owner}/{repo}");
        self.aliases.insert(
            repo.to_string(),
            Package {
                owner: owner.to_string(),
                repo: repo.to_string(),
            },
        );
        println!("{}: {}", pkg_name, release.tag_name);
        self.releases.insert(pkg_name, release.tag_name);
        Ok(())
    }

    pub async fn update(&mut self) -> Result<()> {
        let octocrab = octocrab::instance();

        let mut futures = Vec::new();
        for pkg in self.aliases.values() {
            let octocrab = octocrab.clone();
            let pkg = pkg.clone();

            let future: JoinHandle<Result<(String, String)>> = tokio::spawn(async move {
                let release = octocrab
                    .repos(&pkg.owner, &pkg.repo)
                    .releases()
                    .get_latest()
                    .await?;
                Ok((pkg.to_string(), release.tag_name))
            });

            futures.push(future);
        }

        for result in join_all(futures).await {
            match result? {
                Ok((pkg_name, tag_name)) => {
                    let existing = self
                        .releases
                        .get(&pkg_name)
                        .with_context(|| format!("Package {pkg_name} not found"))?;
                    if existing == &tag_name {
                        continue;
                    }
                    println!("{pkg_name}: {existing} -> {tag_name}");
                    self.releases.insert(pkg_name, tag_name);
                }
                Err(e) => {
                    eprintln!("Failed to update package: {e}");
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CpmInfo {
    pub version: String,
    pub sha256: String,
}

impl CpmInfo {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let content = std::fs::read_to_string(path)?;
        let cpm_info: CpmInfo = serde_json::from_str(&content)?;
        Ok(cpm_info)
    }

    pub fn save(&self, path: impl Into<PathBuf>) -> Result<()> {
        let path = path.into();
        std::fs::write(path, serde_json::to_string(self)?)?;
        Ok(())
    }

    pub async fn query_from_github() -> Result<Self> {
        let octocrab = octocrab::instance();

        let release = octocrab
            .repos("cpm-cmake", "CPM.cmake")
            .releases()
            .get_latest()
            .await?;

        let tag = release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name);

        let asset = release
            .assets
            .first()
            .with_context(|| "No assets found in release")?;

        let content = reqwest::get(asset.browser_download_url.clone())
            .await?
            .bytes()
            .await?;

        let mut hasher = sha2::Sha256::new();
        hasher.update(&content);
        let sha256 = hasher.finalize();

        Ok(CpmInfo {
            version: tag.to_string(),
            sha256: format!("{sha256:x}"),
        })
    }
}
