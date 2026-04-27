use std::process::Command;

use anyhow::{Result, bail};
use cmk_core::store::Store;

pub async fn run(version: &str, rest: &[String]) -> Result<()> {
    if rest.is_empty() {
        bail!("usage: cmk toolchain exec <version> -- <cmd> [args...]");
    }
    let store = Store::open()?;
    let plat = cmk_core::platform::current_platform()?;
    let bin = store.version_dir(version, &plat).join("bin").join(&rest[0]);
    if !bin.exists() {
        bail!("{} not found in version {version}", rest[0]);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = Command::new(&bin).args(&rest[1..]).exec();
        bail!("exec {bin:?}: {err}");
    }
    #[cfg(not(unix))]
    {
        let status = Command::new(&bin).args(&rest[1..]).status()?;
        std::process::exit(status.code().unwrap_or(127));
    }
}
