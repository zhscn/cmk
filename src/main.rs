use std::{
    collections::HashMap,
    num::NonZero,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use cmk::{
    CMakeProject, PackageIndex, Target, completing_read,
    default::{CLANG_FORMAT_CONFIG, CLANG_TIDY_CONFIG, CMAKE_LISTS, GIT_IGNORE, MAIN_CC},
    get_project_root,
};
use serde::{Deserialize, Serialize};
use sha2::Digest;

#[derive(Debug, clap::Parser)]
#[command(version, about)]
struct Cli {
    #[clap(subcommand)]
    command: Option<SubCommand>,
    /// Run the default build command
    #[clap(short, long, value_name = "BUILD_DIR")]
    build: Option<String>,
    /// Run the default build command
    #[clap(short, long, default_value_t = false)]
    interactive: bool,
    /// Run the default build command
    #[clap(short, long)]
    jobs: Option<usize>,
    /// Run the default build command
    #[clap(short, long)]
    target: Option<String>,
}

#[derive(Debug, clap::Subcommand)]
enum SubCommand {
    /// Add a package to the package index
    #[clap(name = "add", visible_alias = "a")]
    Add {
        /// The name of the package with the format of "owner/repo"
        name: String,
    },
    /// Update the package index
    #[clap(name = "update", visible_alias = "u")]
    Update,
    /// Get the cached release of a package in the package index
    #[clap(name = "get", visible_alias = "g")]
    Get {
        /// The name or alias of the package
        name: String,
    },
    /// Create a new project
    #[clap(name = "new", visible_alias = "n")]
    New {
        /// The name of the project
        name: String,
    },
    /// Run the executable target
    #[clap(name = "run", visible_alias = "r")]
    Run {
        /// The path to the build directory relative to the project root
        #[clap(short, long)]
        build: Option<String>,
        /// The name of the executable target
        #[clap(short, long)]
        target: Option<String>,
        /// The arguments to pass to the executable target
        #[clap(last = true)]
        args: Vec<String>,
    },
    /// Build the project
    #[clap(name = "build", visible_alias = "b")]
    Build {
        /// The path to the build directory relative to the project root
        #[clap(short, long)]
        build: Option<String>,
        /// Select the target to build interactively. When the target is
        /// specified, this option is ignored.
        #[clap(short, long, default_value_t = false)]
        interactive: bool,
        /// Run n jobs in parallel
        #[clap(short, long)]
        jobs: Option<usize>,
        /// The name of the executable target
        target: Option<String>,
    },
    /// Build the translation unit
    #[clap(name = "build-tu", visible_alias = "tu")]
    BuildTU {
        /// The path to the build directory relative to the project root
        #[clap(short, long)]
        build: Option<String>,
        /// The name of the translation unit
        name: Option<String>,
    },
    /// Refresh the CMake build directory
    #[clap(name = "refresh", visible_alias = "ref")]
    Refresh {
        /// The path to the build directory relative to the project root
        build: Option<String>,
    },
    /// Format source files with clang-format
    #[clap(name = "fmt", visible_alias = "f")]
    Fmt {
        /// Format all tracked source files
        #[clap(short, long, conflicts_with_all = ["staged", "unstaged"])]
        all: bool,
        /// Format only staged files
        #[clap(short, long, conflicts_with_all = ["all", "unstaged"])]
        staged: bool,
        /// Format only unstaged files
        #[clap(short, long, conflicts_with_all = ["all", "staged"])]
        unstaged: bool,
        /// Print files that would be formatted without modifying them
        #[clap(short, long)]
        dry_run: bool,
        /// Print verbose output
        #[clap(short, long)]
        verbose: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(command) = cli.command {
        match command {
            SubCommand::Add { name } => exec_add(name).await,
            SubCommand::Update => exec_update().await,
            SubCommand::Get { name } => exec_get(name).await,
            SubCommand::New { name } => exec_new(name).await,
            SubCommand::Run {
                target,
                args,
                build,
            } => exec_run(target, args, build),
            SubCommand::Build {
                target,
                build,
                interactive,
                jobs,
            } => exec_build(target, build, interactive, jobs),
            SubCommand::BuildTU { name, build } => exec_build_tu(name, build),
            SubCommand::Refresh { build } => exec_refresh(build),
            SubCommand::Fmt {
                all,
                staged,
                unstaged,
                dry_run,
                verbose,
            } => exec_fmt(all, staged, unstaged, dry_run, verbose),
        }
    } else {
        exec_build(cli.target, cli.build, cli.interactive, cli.jobs)
    }
}

fn get_default_jobs() -> usize {
    std::env::var("CMK_DEFAULT_JOBS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .unwrap_or(NonZero::new(2).unwrap())
                .get()
                - 1
        })
}

// ========== Add command ==========

async fn exec_add(name: String) -> Result<()> {
    let home = std::env::var("HOME")?;
    let pkg_info_path = Path::new(&home).join(".config/cmk/pkg.json");
    let mut index = PackageIndex::load_or_create(&pkg_info_path)?;
    let (owner, repo) = name
        .split_once('/')
        .with_context(|| "Invalid package name")?;
    index.add_repo(owner, repo).await?;
    index.save(&pkg_info_path)?;
    Ok(())
}

// ========== Get command ==========

async fn exec_get(name: String) -> Result<()> {
    let home = std::env::var("HOME")?;
    let pkg_info_path = Path::new(&home).join(".config/cmk/pkg.json");
    let index = PackageIndex::load_or_create(&pkg_info_path)?;
    let pkg_name = index.get_pkg_name(&name)?;
    let release = index.get_release(&pkg_name)?;
    println!("{pkg_name}: {release}");
    Ok(())
}

// ========== Update command ==========

async fn exec_update() -> Result<()> {
    let home = std::env::var("HOME")?;
    let pkg_info_path = Path::new(&home).join(".config/cmk/pkg.json");
    let mut index = PackageIndex::load_or_create(&pkg_info_path)?;
    index.update().await?;
    index.save(&pkg_info_path)?;
    let cpm_info_path = Path::new(&home).join(".config/cmk/cpm.json");
    let old_cpm = CpmInfo::load(&cpm_info_path)?;
    let new_cpm = CpmInfo::query_from_github().await?;
    if old_cpm.version != new_cpm.version {
        println!("CPM: {} -> {}", old_cpm.version, new_cpm.version);
        new_cpm.save(cpm_info_path)?;
    }
    Ok(())
}

// ========== New command ==========

#[derive(Debug, Serialize, Deserialize)]
struct CpmInfo {
    version: String,
    sha256: String,
}

impl CpmInfo {
    fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let content = std::fs::read_to_string(path)?;
        let cpm_info: CpmInfo = serde_json::from_str(&content)?;
        Ok(cpm_info)
    }

    fn save(&self, path: impl Into<PathBuf>) -> Result<()> {
        let path = path.into();
        std::fs::write(path, serde_json::to_string(self)?)?;
        Ok(())
    }

    async fn query_from_github() -> Result<Self> {
        let octocrab = octocrab::instance();

        let release = octocrab
            .repos("cpm-cmake", "CPM.cmake")
            .releases()
            .get_latest()
            .await?;

        let tag = release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name);

        let asset = release
            .assets
            .first()
            .with_context(|| "No assets found in release")?;

        let content = reqwest::get(asset.browser_download_url.clone())
            .await?
            .bytes()
            .await?;

        let mut hasher = sha2::Sha256::new();
        hasher.update(&content);
        let sha256 = hasher.finalize();

        Ok(CpmInfo {
            version: tag.to_string(),
            sha256: format!("{sha256:x}"),
        })
    }
}

async fn exec_new(name: String) -> Result<()> {
    let path = Path::new(&name);
    if path.try_exists()? {
        return Err(anyhow!("{} already exists", name));
    }

    std::fs::create_dir_all(path)?;
    std::env::set_current_dir(path)?;
    std::fs::create_dir_all("src")?;

    std::process::Command::new("git")
        .arg("init")
        .spawn()?
        .wait()?;

    std::fs::write(".gitignore", GIT_IGNORE).unwrap();
    std::fs::write(".clang-format", CLANG_FORMAT_CONFIG).unwrap();
    std::fs::write(".clang-tidy", CLANG_TIDY_CONFIG).unwrap();
    std::fs::write("src/main.cc", MAIN_CC).unwrap();

    let home = std::env::var("HOME")?;
    let cpm_info_path = Path::new(&home).join(".config/cmk/cpm.json");
    let info = if let Ok(info) = CpmInfo::load(&cpm_info_path) {
        info
    } else {
        let parent = cpm_info_path
            .parent()
            .with_context(|| "Failed to get parent directory of cpm.json")?;
        std::fs::create_dir_all(parent)?;
        let info = CpmInfo::query_from_github().await?;
        info.save(&cpm_info_path)?;
        info
    };
    std::fs::write(
        "CMakeLists.txt",
        CMAKE_LISTS
            .replace("{name}", &name)
            .replace("{cpm_version}", &info.version)
            .replace("{cpm_hash_sum}", &info.sha256),
    )?;

    Ok(())
}

// ========== Run command ==========

fn exec_run(target: Option<String>, args: Vec<String>, build: Option<String>) -> Result<()> {
    let project = CMakeProject::new()?;
    let targets = project.collect_executable_targets(build.as_deref())?;
    if targets.is_empty() {
        return Err(anyhow!("Exectuable targets not fount"));
    }
    let targets: HashMap<String, Target> = targets
        .into_iter()
        .map(|target| (target.name.clone(), target))
        .collect();
    let target = if let Some(target) = target {
        targets
            .get(&target)
            .with_context(|| format!("Target {target} not found"))?
    } else {
        let target_names = targets.keys().map(|s| s.to_string()).collect::<Vec<_>>();
        if target_names.len() == 1 {
            targets
                .get(&target_names[0])
                .with_context(|| format!("Target {} not found", target_names[0]))?
        } else {
            let target_name = completing_read(&target_names)?;
            if target_name.is_empty() {
                return Err(anyhow!("No target selected"));
            }
            targets
                .get(&target_name)
                .with_context(|| format!("Target {target_name} not found"))?
        }
    };
    project.run_target(target, &args, None)?;
    Ok(())
}

// ========== Build command ==========

fn exec_build(
    target: Option<String>,
    build: Option<String>,
    interactive: bool,
    jobs: Option<usize>,
) -> Result<()> {
    let project = CMakeProject::new()?;
    let build = if let Some(dir) = build {
        let bp = PathBuf::from(&dir);
        let rp = if bp.is_absolute() {
            bp.strip_prefix(&project.project_root)?.to_owned()
        } else {
            let p = std::env::current_dir()?.join(bp);
            p.strip_prefix(&project.project_root)?.to_owned()
        };
        rp.to_string_lossy().to_string()
    } else {
        let dirs = project.list_build_dirs();
        if dirs.len() == 1 {
            dirs[0].clone()
        } else if let Some(k) = project.detect_pwd_key() {
            k
        } else {
            let res = completing_read(&dirs)?;
            if res.is_empty() {
                return Err(anyhow!("No build directory selected"));
            }
            res
        }
    };
    let target = if interactive && target.is_none() {
        let targets = project.collect_executable_targets(Some(&build))?;
        if targets.is_empty() {
            return Err(anyhow!("No buildable targets found"));
        }
        let target_names = targets.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
        let target_name = completing_read(&target_names)?;
        if target_name.is_empty() {
            return Err(anyhow!("No target selected"));
        }
        target_name
    } else {
        target.unwrap_or_else(|| "all".to_string())
    };
    project.build_target(&target, Some(&build), jobs.unwrap_or_else(get_default_jobs))?;
    Ok(())
}

// ========== BuildTU command ==========

fn exec_build_tu(name: Option<String>, build: Option<String>) -> Result<()> {
    let project = CMakeProject::new()?;
    let tu = if let Some(name) = name {
        name
    } else {
        let tu = project.list_all_translation_units(build.as_deref())?;
        let tu = completing_read(&tu)?;
        if tu.is_empty() {
            return Err(anyhow!("No translation unit selected"));
        }
        tu
    };
    println!("build TU: {tu}");
    project.build_tu(&tu, None)?;
    Ok(())
}
// ========== Refresh command ==========

fn exec_refresh(build: Option<String>) -> Result<()> {
    let project = CMakeProject::new()?;
    project.refresh_build_dir(build.as_deref())?;
    Ok(())
}

// ========== Fmt command ==========

fn is_c_or_cpp(magika: &mut magika::Session, path: &Path) -> bool {
    let Ok(result) = magika.identify_file_sync(path) else {
        return false;
    };
    matches!(result.info().label, "c" | "cpp")
}

fn exec_fmt(all: bool, staged: bool, unstaged: bool, dry_run: bool, verbose: bool) -> Result<()> {
    let project_root = get_project_root()?;

    let git = |args: &[&str]| -> Result<String> {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&project_root)
            .output()?;
        Ok(String::from_utf8(output.stdout)?)
    };

    let output_str = if all {
        git(&["ls-files"])?
    } else if staged {
        git(&["diff", "--name-only", "--cached"])?
    } else if unstaged {
        git(&["diff", "--name-only"])?
    } else {
        // Default: both staged + unstaged vs HEAD
        let output = std::process::Command::new("git")
            .args(["diff", "--name-only", "HEAD"])
            .current_dir(&project_root)
            .output()?;
        if output.status.success() {
            String::from_utf8(output.stdout)?
        } else {
            // Fresh repo with no commits: fall back to --cached
            git(&["diff", "--name-only", "--cached"])?
        }
    };

    let candidates: Vec<PathBuf> = output_str
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| project_root.join(line))
        .filter(|path| path.exists())
        .collect();

    if verbose {
        println!("Found {} candidate file(s).", candidates.len());
    }

    if candidates.is_empty() {
        println!("No source files to format.");
        return Ok(());
    }

    let mut magika = magika::Session::new()?;
    let files: Vec<&PathBuf> = candidates
        .iter()
        .filter(|path| {
            let is_src = is_c_or_cpp(&mut magika, path);
            if verbose && !is_src {
                println!("Skipping (not C/C++): {}", path.display());
            }
            is_src
        })
        .collect();

    if files.is_empty() {
        println!("No source files to format.");
        return Ok(());
    }

    if verbose || dry_run {
        for file in &files {
            println!("{}", file.display());
        }
    }

    if dry_run {
        println!("{} file(s) would be formatted.", files.len());
        return Ok(());
    }

    let jobs = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let chunk_size = files.len().div_ceil(jobs);

    let failed = std::sync::atomic::AtomicBool::new(false);
    crossbeam::scope(|s| {
        for chunk in files.chunks(chunk_size) {
            let failed = &failed;
            let project_root = &project_root;
            s.spawn(move |_| {
                let ret = std::process::Command::new("clang-format")
                    .arg("-i")
                    .args(chunk)
                    .current_dir(project_root)
                    .spawn()
                    .and_then(|mut child| child.wait());
                if !matches!(ret, Ok(status) if status.success()) {
                    failed.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            });
        }
    })
    .map_err(|_| anyhow!("clang-format thread panicked"))?;

    if failed.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(anyhow!("clang-format failed"));
    }

    println!("Formatted {} file(s).", files.len());
    Ok(())
}
