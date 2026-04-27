use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::container::ContainerSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Os {
    Linux,
    Macos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arch {
    X86_64,
    Aarch64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub os: Os,
    pub arch: Arch,
}

impl Target {
    pub fn platform_key(&self) -> &'static str {
        match (self.os, self.arch) {
            (Os::Linux, Arch::X86_64) => "linux-x86_64",
            (Os::Linux, Arch::Aarch64) => "linux-aarch64",
            (Os::Macos, Arch::Aarch64) => "darwin-arm64",
            (Os::Macos, Arch::X86_64) => "darwin-x86_64",
        }
    }
    pub fn llvm_target(&self) -> &'static str {
        match self.arch {
            Arch::X86_64 => "X86",
            Arch::Aarch64 => "AArch64",
        }
    }
    pub fn triple(&self) -> &'static str {
        match (self.os, self.arch) {
            (Os::Linux, Arch::X86_64) => "x86_64-unknown-linux-gnu",
            (Os::Linux, Arch::Aarch64) => "aarch64-unknown-linux-gnu",
            (Os::Macos, Arch::Aarch64) => "arm64-apple-darwin",
            (Os::Macos, Arch::X86_64) => "x86_64-apple-darwin",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Baseline {
    El7,
    El8,
    MacOS13,
}

impl Baseline {
    /// Minimum glibc symver this baseline allows. None for non-glibc.
    pub fn min_glibc(&self) -> Option<&'static str> {
        match self {
            Baseline::El7 => Some("2.17"),
            Baseline::El8 => Some("2.28"),
            Baseline::MacOS13 => None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Baseline::El7 => "el7",
            Baseline::El8 => "el8",
            Baseline::MacOS13 => "macos-13",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostToolchain {
    SystemGcc { gcc_prefix: PathBuf, version: String },
    Clangup { version: String, path: PathBuf },
    External { cc: PathBuf, cxx: PathBuf, sysroot: Option<PathBuf> },
}

impl HostToolchain {
    pub fn provides_libcxx(&self) -> bool {
        matches!(self, HostToolchain::Clangup { .. })
    }
    pub fn min_glibc(&self) -> Option<String> {
        // Resolved by stages.rs after host self-check (design §7.5).
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CxxStdlib {
    Bundled,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unwinder {
    Libgcc,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapStage {
    pub source_dir: PathBuf,
    pub build_dir: PathBuf,
    pub install_dir: PathBuf,
    pub extra_cmake: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalStage {
    pub source_dir: PathBuf,
    pub build_dir: PathBuf,
    pub install_dir: PathBuf,
    pub projects: Vec<String>,
    pub runtimes: Vec<String>,
    pub extra_cmake: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub version: String,
    pub target: Target,
    pub baseline: Baseline,
    pub host: HostToolchain,
    pub bootstrap: Option<BootstrapStage>,
    pub final_build: FinalStage,
    pub compiler_rt_pass2: bool,
    pub container: Option<ContainerSpec>,
    pub cxx_stdlib: CxxStdlib,
    pub unwinder: Unwinder,
}
