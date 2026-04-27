use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::platform::install_id;

/// Filesystem layout described in design §4.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn root_from_env() -> Result<PathBuf> {
        if let Ok(s) = std::env::var("CMK_HOME") {
            return Ok(PathBuf::from(s));
        }
        let home = dirs::home_dir().ok_or_else(|| {
            Error::Other(anyhow::anyhow!(
                "cannot resolve $HOME (set CMK_HOME explicitly)"
            ))
        })?;
        Ok(home.join(".cmk"))
    }

    pub fn open() -> Result<Self> {
        Ok(Self {
            root: Self::root_from_env()?,
        })
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    /// User config lives under XDG config (`~/.config/cmk/config.toml`),
    /// not under the state root. See design §3.1.
    pub fn config_path() -> Result<PathBuf> {
        let base = dirs::config_dir().ok_or_else(|| {
            Error::Other(anyhow::anyhow!(
                "cannot resolve $XDG_CONFIG_HOME / $HOME for config path"
            ))
        })?;
        Ok(base.join("cmk").join("config.toml"))
    }
    pub fn installed_path(&self) -> PathBuf {
        self.root.join("installed.json")
    }
    pub fn current_path(&self) -> PathBuf {
        self.root.join("current")
    }
    pub fn toolchains_dir(&self) -> PathBuf {
        self.root.join("toolchains")
    }
    pub fn shims_dir(&self) -> PathBuf {
        self.root.join("shims")
    }
    pub fn manifests_cache(&self) -> PathBuf {
        self.root.join("manifests/cache")
    }
    pub fn downloads(&self) -> PathBuf {
        self.root.join("downloads")
    }
    pub fn build_cache(&self) -> PathBuf {
        self.root.join("build-cache")
    }
    pub fn ccache(&self) -> PathBuf {
        self.root.join("ccache")
    }
    pub fn host_deps(&self) -> PathBuf {
        self.root.join("host-deps")
    }
    /// Project-dep source cache root. See design §3.2.
    pub fn dep_cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }
    pub fn dep_tarball_dir(&self) -> PathBuf {
        self.dep_cache_dir().join("tarball")
    }
    pub fn dep_src_dir(&self) -> PathBuf {
        self.dep_cache_dir().join("src")
    }

    pub fn version_dir(&self, version: &str, platform: &str) -> PathBuf {
        self.toolchains_dir().join(install_id(version, platform))
    }

    pub fn version_meta_dir(&self, version: &str, platform: &str) -> PathBuf {
        self.version_dir(version, platform).join(".cmk")
    }

    pub fn ensure_skeleton(&self) -> Result<()> {
        for d in [
            self.toolchains_dir(),
            self.shims_dir(),
            self.manifests_cache(),
            self.downloads(),
            self.build_cache(),
        ] {
            fs::create_dir_all(d)?;
        }
        Ok(())
    }

    pub fn read_installed(&self) -> Result<InstalledIndex> {
        let p = self.installed_path();
        if !p.exists() {
            return Ok(InstalledIndex::default());
        }
        let text = fs::read_to_string(&p)?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn write_installed(&self, idx: &InstalledIndex) -> Result<()> {
        let p = self.installed_path();
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = p.with_extension("json.tmp");
        let mut f = fs::File::create(&tmp)?;
        f.write_all(serde_json::to_string_pretty(idx)?.as_bytes())?;
        f.sync_all()?;
        fs::rename(tmp, p)?;
        Ok(())
    }

    pub fn read_current(&self) -> Result<Option<String>> {
        let p = self.current_path();
        if !p.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read_to_string(p)?.trim().to_string()))
    }

    pub fn write_current(&self, version: &str) -> Result<()> {
        fs::create_dir_all(self.root())?;
        let tmp = self.current_path().with_extension("tmp");
        fs::write(&tmp, format!("{version}\n"))?;
        fs::rename(tmp, self.current_path())?;
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct InstalledIndex {
    /// Keyed by `<version>-<platform>`.
    #[serde(default)]
    pub versions: BTreeMap<String, InstalledVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledVersion {
    pub version: String,
    pub platform: String,
    #[serde(default)]
    pub packages: BTreeMap<String, InstalledPackage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub sha256: String,
    pub installed_at: String,
    #[serde(default)]
    pub files: Vec<String>,
}
