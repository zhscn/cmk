use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub enum SourceSpec {
    /// http(s) URL to the LLVM monorepo source tarball.
    Url(String),
    /// Pre-extracted llvm-project tree (root containing `llvm/`, `clang/`, ...).
    Local(PathBuf),
}

impl SourceSpec {
    pub fn default_for_version(version: &str) -> Self {
        Self::Url(format!(
            "https://github.com/llvm/llvm-project/releases/download/llvmorg-{version}/llvm-project-{version}.src.tar.xz",
        ))
    }
}

/// Resolve a SourceSpec into an extracted tree on disk. `into_dir`
/// receives the llvm-project root. Returns provenance (URL + sha256)
/// for the URL case so callers can embed it in the build manifest.
pub fn prepare(
    spec: &SourceSpec,
    into_dir: &Path,
    downloads: &Path,
) -> Result<Option<crate::provenance::SourceProvenance>> {
    match spec {
        SourceSpec::Local(p) => {
            anyhow::ensure!(
                p.join("llvm").is_dir(),
                "{p:?} doesn't look like an llvm-project tree (no `llvm/`)"
            );
            std::fs::create_dir_all(into_dir)?;
            symlink_force(p, into_dir)?;
            Ok(None)
        }
        SourceSpec::Url(url) => {
            std::fs::create_dir_all(downloads)?;
            let fname = url
                .rsplit('/')
                .next()
                .unwrap_or("llvm-project.tar.xz")
                .to_string();
            let local = cmk_toolchain::fetch::fetch_to_blocking(url, downloads, &fname)?;
            let sha = cmk_toolchain::extract::sha256_file(&local)?;
            std::fs::create_dir_all(into_dir)?;
            cmk_toolchain::extract::extract_tar_auto(&local, into_dir)
                .with_context(|| format!("extract {local:?}"))?;
            normalize_extracted(into_dir)?;
            Ok(Some(crate::provenance::SourceProvenance {
                url: url.clone(),
                sha256: sha,
            }))
        }
    }
}

fn normalize_extracted(into_dir: &Path) -> Result<()> {
    if into_dir.join("llvm").is_dir() {
        return Ok(());
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    for e in std::fs::read_dir(into_dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            roots.push(e.path());
        }
    }
    if roots.len() == 1 {
        let single = roots.pop().unwrap();
        if single.join("llvm").is_dir() {
            for e in std::fs::read_dir(&single)? {
                let e = e?;
                let dst = into_dir.join(e.file_name());
                std::fs::rename(e.path(), &dst)?;
            }
            std::fs::remove_dir(&single).ok();
            return Ok(());
        }
    }
    anyhow::bail!("extracted source has no `llvm/` directory under {into_dir:?}")
}

#[cfg(unix)]
fn symlink_force(target: &Path, link_root: &Path) -> Result<()> {
    // Replace `link_root` (an empty dir) with a symlink to `target`.
    if link_root.exists() {
        if link_root.is_dir() && std::fs::read_dir(link_root)?.next().is_none() {
            std::fs::remove_dir(link_root)?;
        } else if link_root.is_symlink() {
            std::fs::remove_file(link_root)?;
        } else {
            anyhow::bail!("{link_root:?} exists and isn't an empty dir we can replace");
        }
    }
    std::os::unix::fs::symlink(target, link_root)?;
    Ok(())
}

#[cfg(not(unix))]
fn symlink_force(_target: &Path, _link_root: &Path) -> Result<()> {
    anyhow::bail!("non-unix builder host not supported")
}
