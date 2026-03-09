use std::{
    collections::HashMap,
    num::NonZero,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use cmk::{
    CMakeProject, CpmInfo, FmtConfig, PackageIndex, Target, completing_read,
    default::load_template, get_project_root,
};
use tokio::process::Command;

pub(crate) fn get_default_jobs() -> usize {
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

pub(crate) async fn exec_add(name: String) -> Result<()> {
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

pub(crate) async fn exec_get(name: String) -> Result<()> {
    let home = std::env::var("HOME")?;
    let pkg_info_path = Path::new(&home).join(".config/cmk/pkg.json");
    let index = PackageIndex::load_or_create(&pkg_info_path)?;
    let pkg_name = index.get_pkg_name(&name)?;
    let release = index.get_release(&pkg_name)?;
    println!("{pkg_name}: {release}");
    Ok(())
}

// ========== Update command ==========

pub(crate) async fn exec_update() -> Result<()> {
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

pub(crate) async fn exec_new(name: String, template: Option<String>) -> Result<()> {
    let path = Path::new(&name);
    if path.try_exists()? {
        return Err(anyhow!("{} already exists", name));
    }

    let template = load_template(template.as_deref()).await?;

    std::fs::create_dir_all(path)?;
    std::env::set_current_dir(path)?;

    Command::new("git").arg("init").spawn()?.wait().await?;

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

    let mut vars = HashMap::new();
    vars.insert("{name}", name.as_str());
    vars.insert("{cpm_version}", info.version.as_str());
    vars.insert("{cpm_hash_sum}", info.sha256.as_str());

    let project_dir = std::env::current_dir()?;
    template.apply(&project_dir, &vars)?;

    Ok(())
}

// ========== Run command ==========

pub(crate) async fn exec_run(
    target: Option<String>,
    args: Vec<String>,
    build: Option<String>,
) -> Result<()> {
    let project = CMakeProject::new().await?;
    let targets = project.collect_executable_targets(build.as_deref()).await?;
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
            let target_name = completing_read(&target_names).await?;
            if target_name.is_empty() {
                return Err(anyhow!("No target selected"));
            }
            targets
                .get(&target_name)
                .with_context(|| format!("Target {target_name} not found"))?
        }
    };
    project.run_target(target, &args, None).await?;
    Ok(())
}

// ========== Build command ==========

pub(crate) async fn exec_build(
    target: Option<String>,
    build: Option<String>,
    interactive: bool,
    jobs: Option<usize>,
) -> Result<()> {
    let project = CMakeProject::new().await?;
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
            let res = completing_read(&dirs).await?;
            if res.is_empty() {
                return Err(anyhow!("No build directory selected"));
            }
            res
        }
    };
    let target = if interactive && target.is_none() {
        let targets = project.collect_executable_targets(Some(&build)).await?;
        if targets.is_empty() {
            return Err(anyhow!("No buildable targets found"));
        }
        let target_names = targets.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
        let target_name = completing_read(&target_names).await?;
        if target_name.is_empty() {
            return Err(anyhow!("No target selected"));
        }
        target_name
    } else {
        target.unwrap_or_else(|| "all".to_string())
    };
    project
        .build_target(&target, Some(&build), jobs.unwrap_or_else(get_default_jobs))
        .await?;
    Ok(())
}

// ========== BuildTU command ==========

pub(crate) async fn exec_build_tu(name: Option<String>, build: Option<String>) -> Result<()> {
    let project = CMakeProject::new().await?;
    let tu = if let Some(name) = name {
        name
    } else {
        let tu = project.list_all_translation_units(build.as_deref()).await?;
        let tu = completing_read(&tu).await?;
        if tu.is_empty() {
            return Err(anyhow!("No translation unit selected"));
        }
        tu
    };
    println!("build TU: {tu}");
    project.build_tu(&tu, None).await?;
    Ok(())
}

// ========== Refresh command ==========

pub(crate) async fn exec_refresh(build: Option<String>) -> Result<()> {
    let project = CMakeProject::new().await?;
    project.refresh_build_dir(build.as_deref()).await?;
    Ok(())
}

// ========== Fmt command ==========

fn is_c_or_cpp(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(
            "c" | "h"
                | "cc"
                | "cpp"
                | "cxx"
                | "c++"
                | "hh"
                | "hpp"
                | "hxx"
                | "h++"
                | "ixx"
                | "cppm"
                | "ccm"
                | "cxxm"
                | "c++m"
                | "mxx"
                | "mpp",
        )
    )
}

pub(crate) async fn exec_fmt(
    all: bool,
    staged: bool,
    unstaged: bool,
    dry_run: bool,
    verbose: bool,
) -> Result<()> {
    let project_root = get_project_root().await?;

    async fn run_git(args: &[&str], project_root: &Path) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(project_root)
            .output()
            .await?;
        Ok(String::from_utf8(output.stdout)?)
    }

    let output_str = if all {
        run_git(&["ls-files"], &project_root).await?
    } else if staged {
        run_git(&["diff", "--name-only", "--cached"], &project_root).await?
    } else if unstaged {
        run_git(&["diff", "--name-only"], &project_root).await?
    } else {
        // Default: both staged + unstaged vs HEAD
        let output = Command::new("git")
            .args(["diff", "--name-only", "HEAD"])
            .current_dir(&project_root)
            .output()
            .await?;
        if output.status.success() {
            String::from_utf8(output.stdout)?
        } else {
            // Fresh repo with no commits: fall back to --cached
            run_git(&["diff", "--name-only", "--cached"], &project_root).await?
        }
    };

    let candidates: Vec<PathBuf> = output_str
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| project_root.join(line))
        .filter(|path| path.exists())
        .collect();

    // Filter out ignored files based on .cmk.toml [fmt] ignore patterns
    let fmt_config = FmtConfig::load(&project_root)?;
    let candidates = if fmt_config.ignore.is_empty() {
        candidates
    } else {
        let mut builder = globset::GlobSetBuilder::new();
        for pattern in &fmt_config.ignore {
            builder.add(globset::Glob::new(pattern)?);
        }
        let ignore_set = builder.build()?;
        candidates
            .into_iter()
            .filter(|path| {
                let rel = path.strip_prefix(&project_root).unwrap_or(path);
                let ignored = ignore_set.is_match(rel);
                if verbose && ignored {
                    println!("Skipping (ignored): {}", rel.display());
                }
                !ignored
            })
            .collect()
    };

    if verbose {
        println!("Found {} candidate file(s).", candidates.len());
    }

    if candidates.is_empty() {
        println!("No source files to format.");
        return Ok(());
    }

    let files: Vec<PathBuf> = candidates
        .into_iter()
        .filter(|path| {
            let is_src = is_c_or_cpp(path);
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

    if verbose {
        for file in &files {
            println!("{}", file.display());
        }
    }

    let jobs = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let chunk_size = files.len().div_ceil(jobs);

    if dry_run {
        let unformatted = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
        let mut handles = Vec::new();
        for chunk in files.chunks(chunk_size) {
            let chunk: Vec<PathBuf> = chunk.to_vec();
            let project_root = project_root.clone();
            let unformatted = Arc::clone(&unformatted);
            handles.push(tokio::spawn(async move {
                for file in &chunk {
                    let ret = Command::new("clang-format")
                        .args(["--dry-run", "-Werror"])
                        .arg(file)
                        .current_dir(&project_root)
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .output()
                        .await;
                    if !matches!(ret, Ok(ref output) if output.status.success()) {
                        unformatted.lock().await.push(file.display().to_string());
                    }
                }
            }));
        }
        for handle in handles {
            handle.await?;
        }

        let unformatted = Arc::try_unwrap(unformatted)
            .expect("all tasks joined")
            .into_inner();
        if unformatted.is_empty() {
            return Ok(());
        }
        for file in &unformatted {
            println!("{file}");
        }
        return Err(anyhow!("{} file(s) need formatting.", unformatted.len()));
    }

    let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let changed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut handles = Vec::new();
    for chunk in files.chunks(chunk_size) {
        let chunk: Vec<PathBuf> = chunk.to_vec();
        let project_root = project_root.clone();
        let failed = Arc::clone(&failed);
        let changed = Arc::clone(&changed);
        handles.push(tokio::spawn(async move {
            let mut contents_before = Vec::with_capacity(chunk.len());
            for file in &chunk {
                match tokio::fs::read(file).await {
                    Ok(bytes) => contents_before.push(bytes),
                    Err(_) => {
                        failed.store(true, std::sync::atomic::Ordering::Relaxed);
                        return;
                    }
                }
            }
            let ret = Command::new("clang-format")
                .arg("-i")
                .args(&chunk)
                .current_dir(&project_root)
                .output()
                .await;
            if !matches!(ret, Ok(ref output) if output.status.success()) {
                failed.store(true, std::sync::atomic::Ordering::Relaxed);
                return;
            }
            let mut count = 0usize;
            for (file, before) in chunk.iter().zip(contents_before.iter()) {
                match tokio::fs::read(file).await {
                    Ok(after) => {
                        if after != *before {
                            count += 1;
                        }
                    }
                    Err(_) => {
                        failed.store(true, std::sync::atomic::Ordering::Relaxed);
                        return;
                    }
                }
            }
            changed.fetch_add(count, std::sync::atomic::Ordering::Relaxed);
        }));
    }
    for handle in handles {
        handle.await?;
    }

    if failed.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(anyhow!("clang-format failed"));
    }

    let changed = changed.load(std::sync::atomic::Ordering::Relaxed);
    println!("Formatted {} file(s), {} changed.", files.len(), changed);
    Ok(())
}
