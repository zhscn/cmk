//! Project-level configuration loaded from `.cmk.toml`.
//!
//! Schema overview (design.md §4):
//! - `schema = 2`             — top-level version marker (M4+ enforces)
//! - `[toolchain]`            — clang toolchain selection (M4 wires to cmk-toolchain)
//! - `[deps.cmake]`           — declarative CMake-recipe deps (M5)
//! - `[deps.custom]`          — build.sh-driven deps (M7)
//! - `[build]`                — build dir defaults
//! - `[fmt]` / `[lint]`       — clang-format / clang-tidy filters
//!
//! `[vars]` / `[env]` / `[env.*]` are **deleted** vs. schema=1; they were the
//! manual `${DEPS_INSTALL}` glue replaced by automatic env injection from
//! `[toolchain]` + `[deps.*]`. See design.md §4.1.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;

const CONFIG_FILE_NAME: &str = ".cmk.toml";

/// Top-level `.cmk.toml` schema. All sections optional.
#[derive(Debug, Deserialize, Default)]
pub struct CmkConfig {
    /// Version marker. M4+ rejects `schema = 1` configs that still carry
    /// `[vars]` / `[env]`.
    #[serde(default)]
    pub schema: Option<u32>,
    #[serde(default)]
    pub toolchain: Option<ToolchainSection>,
    #[serde(default)]
    pub deps: Option<DepsSection>,
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub fmt: FmtConfig,
    #[serde(default)]
    pub lint: LintConfig,
}

impl CmkConfig {
    /// Load `.cmk.toml` from `project_root`. Returns default if absent.
    pub fn load(project_root: &Path) -> Result<Self> {
        let path = project_root.join(CONFIG_FILE_NAME);
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let cfg: Self = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(cfg)
    }

    pub fn exists(project_root: &Path) -> bool {
        project_root.join(CONFIG_FILE_NAME).exists()
    }
}

/// `[toolchain]` section. Wired in M4 to cmk-toolchain.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct ToolchainSection {
    /// `"18.1.8"` (cmk-managed) | `"system"` | `{ path = "/opt/clang" }`.
    /// M4 parses the variants; for now stored as raw value.
    #[serde(default, rename = "use")]
    pub use_: Option<toml::Value>,
}

/// `[deps]` section. Each subtable is a flat map `name -> spec`.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct DepsSection {
    #[serde(default)]
    pub cmake: HashMap<String, toml::Value>,
    #[serde(default)]
    pub custom: HashMap<String, toml::Value>,
}

/// `[build]` section.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct BuildConfig {
    /// Default build dir relative to project root.
    #[serde(default)]
    pub default: Option<String>,
}

/// `[fmt]` section.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct FmtConfig {
    #[serde(default)]
    pub ignore: Vec<String>,
}

/// `[lint]` section.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct LintConfig {
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default)]
    pub warnings_as_errors: bool,
    #[serde(default)]
    pub header_filter: Option<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

// Convenience load fns kept for callers that only need one section.

impl BuildConfig {
    pub fn load(project_root: &Path) -> Result<Self> {
        Ok(CmkConfig::load(project_root)?.build)
    }
}

impl FmtConfig {
    pub fn load(project_root: &Path) -> Result<Self> {
        Ok(CmkConfig::load(project_root)?.fmt)
    }
}

impl LintConfig {
    pub fn load(project_root: &Path) -> Result<Self> {
        Ok(CmkConfig::load(project_root)?.lint)
    }
}

/// Environment injected into cmake / ninja / target binary invocations.
///
/// **M0 placeholder**: empty. M4 fills `CC`/`CXX`/`PATH` prefix from
/// `[toolchain]`; M5 fills `CMAKE_PREFIX_PATH`/`PKG_CONFIG_PATH`/`LD_LIBRARY_PATH`
/// from `[deps.*]` install prefix.
#[derive(Debug, Default, Clone)]
pub struct BuildEnv {
    project_root: PathBuf,
}

impl BuildEnv {
    pub fn load(project_root: &Path) -> Result<Self> {
        let _ = CmkConfig::load(project_root)?;
        Ok(Self {
            project_root: project_root.to_path_buf(),
        })
    }

    /// Env vars for build commands (cmake/ninja). M0: empty.
    pub fn build_env(&self, _build_dir: Option<&Path>) -> HashMap<String, String> {
        HashMap::new()
    }

    /// Env vars for `cmk run <target>`. M0: empty.
    pub fn run_env(
        &self,
        _target_name: Option<&str>,
        _build_dir: Option<&Path>,
    ) -> HashMap<String, String> {
        HashMap::new()
    }

    pub fn apply_to_command(
        &self,
        cmd: &mut tokio::process::Command,
        env: &HashMap<String, String>,
    ) {
        for (key, value) in env {
            cmd.env(key, value);
        }
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }
}
