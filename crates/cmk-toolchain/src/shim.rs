use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cmk_core::store::{InstalledIndex, Store};

/// Recompute `~/.cmk/shims/` so it covers exactly the union of
/// binaries provided by every installed version. Each shim is a
/// symlink to the single `cmk-shim` dispatcher binary, which reads
/// `argv[0]` and execs the right per-version executable.
pub fn rebuild_shims(store: &Store, shim_bin: &Path) -> Result<()> {
    let shims_dir = store.shims_dir();
    fs::create_dir_all(&shims_dir)?;

    let names = collect_bin_names(&store.read_installed()?, store)?;

    // Drop stale shims that no longer correspond to any installed bin.
    if let Ok(rd) = fs::read_dir(&shims_dir) {
        for entry in rd.flatten() {
            let n = entry.file_name();
            let s = n.to_string_lossy().to_string();
            if !names.contains(&s) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    for name in &names {
        let link = shims_dir.join(name);
        if link.exists() {
            let _ = fs::remove_file(&link);
        }
        symlink(shim_bin, &link)
            .with_context(|| format!("symlink {shim_bin:?} -> {link:?}"))?;
    }
    Ok(())
}

fn collect_bin_names(idx: &InstalledIndex, store: &Store) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    for inst in idx.versions.values() {
        let bin_dir = store
            .version_dir(&inst.version, &inst.platform)
            .join("bin");
        if !bin_dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&bin_dir)?.flatten() {
            if let Ok(ft) = entry.file_type()
                && (ft.is_file() || ft.is_symlink())
                && let Some(n) = entry.file_name().to_str()
            {
                out.insert(n.to_string());
            }
        }
    }
    Ok(out)
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "windows is not a supported cmk platform",
    ))
}

pub fn locate_shim_binary(cli_self: &Path) -> PathBuf {
    // Convention: shipped alongside the cmk CLI binary.
    if let Some(dir) = cli_self.parent() {
        let p = dir.join(if cfg!(windows) {
            "cmk-shim.exe"
        } else {
            "cmk-shim"
        });
        if p.exists() {
            return p;
        }
    }
    PathBuf::from(if cfg!(windows) {
        "cmk-shim.exe"
    } else {
        "cmk-shim"
    })
}
