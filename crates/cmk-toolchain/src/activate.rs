use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use cmk_core::store::Store;

/// Verify a version is installed and write `~/.cmk/current`.
pub fn activate(store: &Store, version: &str) -> Result<()> {
    let plat = cmk_core::platform::current_platform()?;
    let prefix = store.version_dir(version, &plat);
    if !prefix.is_dir() {
        bail!("version `{version}` not installed for {plat}");
    }
    store
        .write_current(version)
        .with_context(|| format!("write {:?}", store.current_path()))?;
    Ok(())
}

/// Resolve `<prefix>/bin/<name>` for the currently-selected version.
pub fn which(store: &Store, version: &str, bin: &str) -> Result<PathBuf> {
    let plat = cmk_core::platform::current_platform()?;
    let p = store.version_dir(version, &plat).join("bin").join(bin);
    if !p.exists() {
        bail!("{bin} not found in {p:?}");
    }
    Ok(p)
}
