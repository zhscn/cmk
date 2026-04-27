use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cmk_core::manifest::{Manifest, Package, Platform, Release};

use crate::package::TarballInfo;

#[derive(Debug, Clone)]
pub struct SourceProvenance {
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct Provenance {
    pub cmk_version: String,
    pub cmk_git_sha: String,
    pub source: Option<SourceProvenance>,
    /// Container image reference used (Linux only). For macOS this is empty.
    pub builder_base: Option<String>,
}

impl Provenance {
    pub fn current() -> Self {
        Self {
            cmk_version: env!("CARGO_PKG_VERSION").to_string(),
            cmk_git_sha: option_env!("CMK_GIT_SHA")
                .unwrap_or("unknown")
                .to_string(),
            source: None,
            builder_base: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlatformDescriptor {
    pub key: String,
    pub baseline: String,
    pub host_glibc_min: Option<String>,
    pub system_libcxx: bool,
    pub system_unwinder: bool,
}

/// Write the per-platform manifest fragment that an aggregator (M7
/// release pipeline) will later merge into the full manifest.toml.
pub fn write_manifest_fragment(
    output_dir: &Path,
    version: &str,
    plat: &PlatformDescriptor,
    tarballs: &[TarballInfo],
    provenance: &Provenance,
) -> Result<PathBuf> {
    use std::collections::BTreeMap;

    std::fs::create_dir_all(output_dir)?;

    let mut packages: BTreeMap<String, Package> = BTreeMap::new();
    for t in tarballs {
        let requires = if t.package == "toolchain" {
            vec![]
        } else {
            vec!["toolchain".to_string()]
        };
        packages.insert(
            t.package.clone(),
            Package {
                url: format!(
                    // Filled in by the aggregator with the real
                    // download URL once everything is uploaded.
                    "TBD://clang-{version}-{}-{}.tar.zst",
                    plat.key, t.package
                ),
                sha256: t.sha256.clone(),
                size: t.size,
                requires,
            },
        );
    }

    let mut platforms: BTreeMap<String, Platform> = BTreeMap::new();
    platforms.insert(
        plat.key.clone(),
        Platform {
            baseline: plat.baseline.clone(),
            host_glibc_min: plat.host_glibc_min.clone(),
            builder_base: provenance.builder_base.clone(),
            system_libcxx: plat.system_libcxx,
            system_unwinder: plat.system_unwinder,
            packages,
        },
    );

    let manifest = Manifest {
        release: Release {
            version: version.into(),
            built: today_utc_date(),
        },
        platform: platforms,
    };
    let mut text = manifest.to_toml()?;
    text.push_str("\n# cmk toolchain build provenance\n");
    text.push_str(&format!("# cmk_version = \"{}\"\n", provenance.cmk_version));
    text.push_str(&format!("# cmk_git_sha = \"{}\"\n", provenance.cmk_git_sha));
    if let Some(s) = &provenance.source {
        text.push_str(&format!("# source_url      = \"{}\"\n", s.url));
        text.push_str(&format!("# source_sha256   = \"{}\"\n", s.sha256));
    }
    if let Some(b) = &provenance.builder_base {
        text.push_str(&format!("# builder_base    = \"{b}\"\n"));
    }

    let path = output_dir.join(format!("manifest.{}.toml", plat.key));
    std::fs::write(&path, text).with_context(|| format!("write {path:?}"))?;
    Ok(path)
}

fn today_utc_date() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = (y + i64::from(m <= 2)) as i32;
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmk_core::manifest::Manifest;

    #[test]
    fn fragment_roundtrips_through_toml() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let plat = PlatformDescriptor {
            key: "linux-x86_64".into(),
            baseline: "el7".into(),
            host_glibc_min: Some("2.17".into()),
            system_libcxx: false,
            system_unwinder: false,
        };
        let tarballs = vec![
            TarballInfo {
                package: "toolchain".into(),
                path: "/x/clang-1.0.0-linux-x86_64-toolchain.tar.zst".into(),
                sha256: "0".repeat(64),
                size: 100,
            },
            TarballInfo {
                package: "devel".into(),
                path: "/x/clang-1.0.0-linux-x86_64-devel.tar.zst".into(),
                sha256: "1".repeat(64),
                size: 200,
            },
        ];
        let prov = Provenance {
            cmk_version: "0.1.0".into(),
            cmk_git_sha: "abc123".into(),
            source: Some(SourceProvenance {
                url: "https://github.com/llvm/llvm-project/releases/download/llvmorg-1.0.0/llvm-project-1.0.0.src.tar.xz".into(),
                sha256: "f".repeat(64),
            }),
            builder_base: Some("ghcr.io/foo/cmk-builder:el7-x86-r1".into()),
        };
        let path = write_manifest_fragment(tmp.path(), "1.0.0", &plat, &tarballs, &prov)?;
        let text = std::fs::read_to_string(&path)?;
        assert!(text.contains("# cmk_git_sha = \"abc123\""));
        assert!(text.contains("# builder_base    = \"ghcr.io/foo/cmk-builder:el7-x86-r1\""));
        let m = Manifest::from_toml(&text)?;
        assert_eq!(m.release.version, "1.0.0");
        let p = m.platform_for("linux-x86_64")?;
        assert_eq!(p.host_glibc_min.as_deref(), Some("2.17"));
        assert_eq!(p.packages["devel"].requires, vec!["toolchain"]);
        Ok(())
    }
}
