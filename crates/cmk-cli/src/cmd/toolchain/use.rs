use anyhow::Result;
use cmk_core::store::Store;
use cmk_toolchain::activate::activate;

pub fn run(version: &str) -> Result<()> {
    let store = Store::open()?;
    activate(&store, version)?;
    println!("active: {version}");
    Ok(())
}
