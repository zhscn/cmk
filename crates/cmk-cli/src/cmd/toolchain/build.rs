use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use cmk_builder::container::ContainerRuntime;
use cmk_builder::recipe::{Arch, Os, Target};
use cmk_builder::{LinuxBuildArgs, MacosBuildArgs, SourceSpec, linux, macos};
use cmk_core::store::Store;

use crate::RuntimeArg;

pub struct BuildArgs {
    pub version: String,
    pub target: Option<String>,
    pub no_container: bool,
    pub output: Option<PathBuf>,
    pub source: Option<String>,
    pub image: Option<String>,
    pub runtime: Option<RuntimeArg>,
    pub shell: bool,
}

pub async fn run(args: BuildArgs) -> Result<()> {
    let target_key = args.target.unwrap_or_else(|| {
        cmk_core::platform::current_platform().unwrap_or_else(|_| "darwin-arm64".into())
    });
    let store = Store::open()?;
    store.ensure_skeleton()?;
    let work_dir = store
        .build_cache()
        .join(format!("{}-{target_key}", args.version));
    std::fs::create_dir_all(&work_dir)?;
    let output_dir = args.output.unwrap_or_else(|| work_dir.join("dist"));
    std::fs::create_dir_all(&output_dir)?;

    let source = match args.source {
        Some(s) if std::path::Path::new(&s).exists() => SourceSpec::Local(PathBuf::from(s)),
        Some(s) => SourceSpec::Url(s),
        None => SourceSpec::default_for_version(&args.version),
    };

    match target_key.as_str() {
        "darwin-arm64" => {
            if args.no_container {
                eprintln!("note: --no-container is implicit on macOS; ignoring");
            }
            let build = MacosBuildArgs {
                version: args.version,
                source,
                work_dir,
                output_dir: output_dir.clone(),
                downloads: store.downloads(),
                jobs: macos::detect_jobs(),
            };
            // Builder internals stay sync (heavy std::process::Command +
            // file IO); offload to a blocking task so the async runtime
            // isn't blocked.
            let outs = tokio::task::spawn_blocking(move || macos::run(&build))
                .await
                .map_err(anyhow::Error::from)?
                .with_context(|| "macOS build pipeline")?;
            print_summary(&output_dir, &outs);
            Ok(())
        }
        "linux-x86_64" | "linux-aarch64" => {
            if args.no_container {
                bail!("--no-container is rejected on Linux (design §6.5)");
            }
            let target = match target_key.as_str() {
                "linux-x86_64" => Target {
                    os: Os::Linux,
                    arch: Arch::X86_64,
                },
                "linux-aarch64" => Target {
                    os: Os::Linux,
                    arch: Arch::Aarch64,
                },
                _ => unreachable!(),
            };
            let image = args.image.ok_or_else(|| {
                anyhow::anyhow!(
                    "--image is required for Linux targets \
                     (e.g. ghcr.io/<org>/cmk-builder:el7-x86-<rev>)"
                )
            })?;
            let runtime = args.runtime.map(|r| match r {
                RuntimeArg::Docker => ContainerRuntime::Docker,
                RuntimeArg::Podman => ContainerRuntime::Podman,
            });
            let build = LinuxBuildArgs {
                version: args.version,
                target,
                source,
                work_dir,
                output_dir: output_dir.clone(),
                downloads: store.downloads(),
                image,
                runtime,
                jobs: macos::detect_jobs(),
                ccache_dir: Some(store.ccache()),
                shell: args.shell,
            };
            let outs = tokio::task::spawn_blocking(move || linux::run(&build))
                .await
                .map_err(anyhow::Error::from)?
                .with_context(|| "Linux build pipeline")?;
            print_summary(&output_dir, &outs);
            Ok(())
        }
        other => bail!("unsupported --target `{other}`"),
    }
}

fn print_summary(output_dir: &std::path::Path, outs: &cmk_builder::BuildOutputs) {
    println!("output: {}", output_dir.display());
    for t in &outs.tarballs {
        println!(
            "  {} {} ({} bytes) {}",
            t.package,
            t.sha256,
            t.size,
            t.path.display()
        );
    }
}
