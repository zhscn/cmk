use anyhow::{Context, Result, bail};
use cmk_core::store::Store;
use cmk_toolchain::shim;

pub async fn run(version: &str) -> Result<()> {
    let store = Store::open()?;
    let plat = cmk_core::platform::current_platform()?;
    let key = format!("{version}-{plat}");

    let mut idx = store.read_installed()?;
    if idx.versions.remove(&key).is_none() {
        bail!("version `{version}` is not installed for {plat}");
    }
    let dir = store.version_dir(version, &plat);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).with_context(|| format!("rm -rf {dir:?}"))?;
    }
    store.write_installed(&idx)?;

    if let Some(cur) = store.read_current()?
        && cur == version
    {
        let _ = std::fs::remove_file(store.current_path());
    }

    let self_exe = std::env::current_exe()?;
    let shim_bin = shim::locate_shim_binary(&self_exe);
    shim::rebuild_shims(&store, &shim_bin)?;

    println!("removed {version} ({plat})");
    Ok(())
}
