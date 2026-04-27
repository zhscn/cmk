use anyhow::Result;
use cmk_core::store::Store;
use cmk_toolchain::activate::which as activate_which;

pub fn run(bin: &str) -> Result<()> {
    let store = Store::open()?;
    let cwd = std::env::current_dir().ok();
    let version = cmk_core::version::resolve(&store, cwd.as_deref())?;
    let p = activate_which(&store, &version, bin)?;
    println!("{}", p.display());
    Ok(())
}
