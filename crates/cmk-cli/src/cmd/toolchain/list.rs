use anyhow::Result;
use cmk_core::config::Config;
use cmk_core::store::Store;

pub async fn run(available: bool) -> Result<()> {
    let store = Store::open()?;
    if available {
        let cfg = Config::load_or_default(&Store::config_path()?)?;
        let idx = cmk_registry::fetch_index_first(&cfg.registries).await?;
        if idx.versions.is_empty() {
            println!("(registry returned an empty index)");
            return Ok(());
        }
        for v in &idx.versions {
            println!("{v}");
        }
        return Ok(());
    }
    let idx = store.read_installed()?;
    let current = store.read_current()?.unwrap_or_default();
    if idx.versions.is_empty() {
        println!("(no versions installed)");
        return Ok(());
    }
    for inst in idx.versions.values() {
        let marker = if inst.version == current { "*" } else { " " };
        let pkgs: Vec<&str> = inst.packages.keys().map(String::as_str).collect();
        println!(
            "{marker} {ver:<14} {plat:<16} [{pkgs}]",
            ver = inst.version,
            plat = inst.platform,
            pkgs = pkgs.join(",")
        );
    }
    Ok(())
}
