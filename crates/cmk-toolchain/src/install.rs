use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use cmk_core::manifest::Manifest;
use cmk_core::store::{InstalledPackage, InstalledVersion, Store};

use crate::extract::{extract_tar_auto, verify_sha256};
use crate::fetch::fetch_to;

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub version: String,
    pub platform: String,
    pub packages: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InstallReport {
    pub installed: Vec<String>,
    pub already_present: Vec<String>,
    pub bin_dir: PathBuf,
}

pub async fn install_packages(
    store: &Store,
    manifest: &Manifest,
    plan: &InstallPlan,
) -> Result<InstallReport> {
    store.ensure_skeleton()?;
    let plat_entry = manifest.platform_for(&plan.platform)?;
    let pkgs = expand_with_requires(plat_entry, &plan.packages)?;

    let prefix = store.version_dir(&plan.version, &plan.platform);
    let meta = store.version_meta_dir(&plan.version, &plan.platform);
    std::fs::create_dir_all(&prefix)?;
    std::fs::create_dir_all(&meta)?;

    let mut idx = store.read_installed()?;
    let key = format!("{}-{}", plan.version, plan.platform);
    let entry = idx.versions.entry(key).or_insert_with(|| InstalledVersion {
        version: plan.version.clone(),
        platform: plan.platform.clone(),
        packages: BTreeMap::new(),
    });

    let mut report = InstallReport {
        installed: vec![],
        already_present: vec![],
        bin_dir: prefix.join("bin"),
    };

    for pkg_name in &pkgs {
        let pkg = plat_entry.package(pkg_name, &plan.platform)?;
        if let Some(existing) = entry.packages.get(pkg_name)
            && existing.sha256.eq_ignore_ascii_case(&pkg.sha256)
        {
            report.already_present.push(pkg_name.clone());
            continue;
        }

        let dl_dir = store.downloads().join(&plan.version);
        let fname = format!("{}-{}-{}.tar.zst", plan.version, plan.platform, pkg_name);
        let local = fetch_to(&pkg.url, &dl_dir, &fname)
            .await
            .with_context(|| format!("fetch package `{pkg_name}` from `{}`", pkg.url))?;

        verify_sha256(&local, &pkg.sha256)
            .with_context(|| format!("verify package `{pkg_name}`"))?;

        let files = extract_tar_auto(&local, &prefix)
            .with_context(|| format!("extract package `{pkg_name}` -> {prefix:?}"))?;

        entry.packages.insert(
            pkg_name.clone(),
            InstalledPackage {
                sha256: pkg.sha256.clone(),
                installed_at: now_rfc3339(),
                files,
            },
        );
        report.installed.push(pkg_name.clone());
    }

    let manifest_copy = meta.join("manifest.toml");
    std::fs::write(&manifest_copy, manifest.to_toml()?)?;
    store.write_installed(&idx)?;

    Ok(report)
}

fn expand_with_requires(
    plat: &cmk_core::manifest::Platform,
    requested: &[String],
) -> Result<Vec<String>> {
    use std::collections::HashSet;
    let mut out: Vec<String> = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = requested.to_vec();
    while let Some(name) = stack.pop() {
        if !plat.packages.contains_key(&name) {
            bail!("package `{name}` not in manifest for this platform");
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        for r in &plat.packages[&name].requires {
            stack.push(r.clone());
        }
        out.push(name);
    }
    // Install dependencies before dependents (toolchain → tools-extra).
    out.reverse();
    Ok(out)
}

fn now_rfc3339() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Tiny inline UTC formatter — keeps us free of the chrono dep.
    let (yr, mo, d, h, mi, s) = epoch_to_ymd_hms(secs as i64);
    format!("{yr:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn epoch_to_ymd_hms(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let mut tod = secs.rem_euclid(86_400) as u32;
    let h = tod / 3600;
    tod %= 3600;
    let mi = tod / 60;
    let s = tod % 60;
    let (y, m, d) = days_to_ymd(days);
    (y, m, d, h, mi, s)
}

fn days_to_ymd(mut z: i64) -> (i32, u32, u32) {
    z += 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + i64::from(m <= 2);
    (y as i32, m as u32, d as u32)
}
