use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Top-level manifest, one per LLVM release. See design §5.1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub release: Release,
    #[serde(default)]
    pub platform: BTreeMap<String, Platform>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub built: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Platform {
    pub baseline: String,
    #[serde(default)]
    pub host_glibc_min: Option<String>,
    #[serde(default)]
    pub builder_base: Option<String>,
    #[serde(default)]
    pub system_libcxx: bool,
    #[serde(default)]
    pub system_unwinder: bool,
    #[serde(default)]
    pub packages: BTreeMap<String, Package>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub requires: Vec<String>,
}

impl Manifest {
    pub fn from_toml(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn platform_for<'a>(&'a self, key: &str) -> Result<&'a Platform> {
        self.platform
            .get(key)
            .ok_or_else(|| Error::PlatformMissing(key.into()))
    }
}

impl Platform {
    pub fn package<'a>(&'a self, name: &str, plat_key: &str) -> Result<&'a Package> {
        self.packages
            .get(name)
            .ok_or_else(|| Error::PackageMissing(name.into(), plat_key.into()))
    }
}
