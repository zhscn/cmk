use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use cmk::{
    CMakeProject, Target, completing_read,
    default::{CLANG_FORMAT_CONFIG, CLANG_TIDY_CONFIG, CMAKE_LISTS, GIT_IGNORE, MAIN_CC},
};
use serde::{Deserialize, Serialize};
use sha2::Digest;

#[derive(Debug, clap::Parser)]
#[command(version, about)]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Create a new project
    #[clap(name = "new")]
    New {
        /// The name of the project
        name: String,
    },
    /// Run the executable target
    #[clap(name = "run")]
    Run {
        /// The name of the executable target
        #[clap(short, long)]
        target: Option<String>,
        /// The arguments to pass to the executable target
        #[clap(last = true)]
        args: Vec<String>,
    },
    /// Build the project
    #[clap(name = "build")]
    Build {
        /// The name of the executable target
        target: Option<String>,
    },
    /// Refresh the CMake build directory
    #[clap(name = "refresh")]
    Refresh,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::New { name } => exec_new(name).await,
        Command::Run { target, args } => exec_run(target, args),
        Command::Build { target } => exec_build(target),
        Command::Refresh => exec_refresh(),
    }
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
            sha256: format!("{:x}", sha256),
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

fn exec_run(target: Option<String>, args: Vec<String>) -> Result<()> {
    let project = CMakeProject::new()?;
    let targets: HashMap<String, Target> = project
        .collect_executable_targets()?
        .into_iter()
        .map(|target| (target.name.clone(), target))
        .collect();
    let target = if let Some(target) = target {
        targets
            .get(&target)
            .with_context(|| format!("Target {} not found", target))?
    } else {
        let target_names = targets.keys().map(|s| s.to_string()).collect::<Vec<_>>();
        let target_name = completing_read(&target_names)?;
        targets
            .get(&target_name)
            .with_context(|| format!("Target {} not found", target_name))?
    };
    project.run_target(target, &args)?;
    Ok(())
}

// ========== Build command ==========

fn exec_build(target: Option<String>) -> Result<()> {
    let project = CMakeProject::new()?;
    project.build_target(target.unwrap_or("all".to_string()).as_str())?;
    Ok(())
}

// ========== Refresh command ==========

fn exec_refresh() -> Result<()> {
    let project = CMakeProject::new()?;
    project.refresh_build_dir()?;
    Ok(())
}
