use std::time::Duration;

use anyhow::{Context, Result, bail};
use cmk_core::config::Config;
use cmk_core::manifest::Manifest;
use cmk_core::store::Store;
use cmk_toolchain::install::{InstallPlan, install_packages};
use cmk_toolchain::shim;

pub async fn run(
    version: Option<String>,
    components: Option<String>,
    manifest: Option<String>,
) -> Result<()> {
    let store = Store::open()?;
    store.ensure_skeleton()?;
    let plat = cmk_core::platform::current_platform()?;

    let (manifest, version) = match manifest {
        Some(spec) => {
            let text = read_manifest_text(&spec).await?;
            let m = Manifest::from_toml(&text)?;
            let v = version.unwrap_or_else(|| m.release.version.clone());
            (m, v)
        }
        None => {
            let v = version.ok_or_else(|| anyhow::anyhow!("missing <version>"))?;
            let cfg = Config::load_or_default(&Store::config_path()?)?;
            let m = cmk_registry::fetch_manifest_any(&cfg.registries, &v).await?;
            cache_manifest(&store, &v, &m)?;
            (m, v)
        }
    };

    let pkgs: Vec<String> = match components {
        Some(s) => s
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => manifest
            .platform_for(&plat)?
            .packages
            .keys()
            .cloned()
            .collect(),
    };
    if pkgs.is_empty() {
        bail!("manifest has no packages for platform `{plat}`");
    }

    let plan = InstallPlan {
        version: version.clone(),
        platform: plat.clone(),
        packages: pkgs,
    };

    let report = install_packages(&store, &manifest, &plan).await?;
    for p in &report.installed {
        println!("installed {version} :: {p}");
    }
    for p in &report.already_present {
        println!("already   {version} :: {p}");
    }

    let self_exe = std::env::current_exe()?;
    let shim_bin = shim::locate_shim_binary(&self_exe);
    shim::rebuild_shims(&store, &shim_bin)?;
    println!("shims     {:?}", store.shims_dir());

    Ok(())
}

async fn read_manifest_text(spec: &str) -> Result<String> {
    if spec.starts_with("http://") || spec.starts_with("https://") {
        return http_get_text(spec).await;
    }
    if let Some(path) = spec.strip_prefix("file://") {
        return tokio::fs::read_to_string(path).await.map_err(Into::into);
    }
    tokio::fs::read_to_string(spec)
        .await
        .with_context(|| format!("read manifest {spec}"))
}

async fn http_get_text(url: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        bail!("GET {url}: HTTP {status}");
    }
    resp.text()
        .await
        .with_context(|| format!("read body of {url}"))
}

fn cache_manifest(store: &Store, version: &str, m: &Manifest) -> Result<()> {
    let dir = store.manifests_cache();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{version}.toml"));
    std::fs::write(&path, m.to_toml()?)
        .with_context(|| format!("write manifest cache {path:?}"))?;
    Ok(())
}
