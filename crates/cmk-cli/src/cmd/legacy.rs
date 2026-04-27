use std::{
    collections::HashMap,
    num::NonZero,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use cmk_config::{FmtConfig, LintConfig};
use cmk_core::{completing_read, confirm};
use cmk_pkg::{CpmInfo, PackageIndex};
use cmk_project::{
    CMakeProject, Target,
    cmake_ast::{CMakeFile, CpmInsertion, render_uri_as_keyword},
    default::load_template,
    get_project_root,
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

pub(crate) async fn exec_add(name: String, project: bool) -> Result<()> {
    let home = std::env::var("HOME")?;
    let pkg_info_path = Path::new(&home).join(".config/cmk/pkg.json");
    let mut index = PackageIndex::load_or_create(&pkg_info_path)?;
    let (owner, repo) = name
        .split_once('/')
        .with_context(|| "Invalid package name")?;
    index.add_repo(owner, repo).await?;
    let tag = index.get_release(&format!("{owner}/{repo}"))?.to_string();
    index.save(&pkg_info_path)?;

    if project {
        insert_cpm_into_cmakelists(owner, repo, &tag).await?;
    }
    Ok(())
}

async fn insert_cpm_into_cmakelists(owner: &str, repo: &str, tag: &str) -> Result<()> {
    let project_root = get_project_root().await?;
    let path = project_root.join("CMakeLists.txt");
    if !path.exists() {
        return Err(anyhow!("CMakeLists.txt not found at {}", path.display()));
    }

    let mut cmake = CMakeFile::parse_path(&path)?;

    let already_present = cmake.cpm_calls().iter().any(|c| {
        c.uri.as_ref().is_some_and(|u| {
            u.source == "gh"
                && u.owner.eq_ignore_ascii_case(owner)
                && u.repo.eq_ignore_ascii_case(repo)
        })
    });
    if already_present {
        println!(
            "{owner}/{repo} is already present in {}; skipping insert.",
            path.display()
        );
        return Ok(());
    }

    let new_call = format!("CPMAddPackage(\"gh:{owner}/{repo}#{tag}\")");
    let insertion = cmake.cpm_insertion();
    let offset = insertion.offset();
    let insert_text = match insertion {
        CpmInsertion::AfterLastCpm(_) => format!("\n{new_call}"),
        CpmInsertion::BeforeFirstTarget(_) => format!("{new_call}\n\n"),
        CpmInsertion::Eof(_) => {
            if cmake.source.ends_with('\n') {
                format!("{new_call}\n")
            } else {
                format!("\n{new_call}\n")
            }
        }
    };

    cmake.splice(offset..offset, &insert_text);
    cmake.save()?;
    println!("Inserted into {}: {new_call}", path.display());
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

pub(crate) async fn exec_update(project: bool, yes: bool) -> Result<()> {
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
    if project {
        update_project_cmakelists(yes).await?;
    }
    Ok(())
}

async fn update_project_cmakelists(yes: bool) -> Result<()> {
    use std::ops::Range;

    let project_root = get_project_root().await?;
    let path = project_root.join("CMakeLists.txt");
    if !path.exists() {
        return Err(anyhow!("CMakeLists.txt not found at {}", path.display()));
    }

    let mut cmake = CMakeFile::parse_path(&path)?;

    struct Pinned {
        owner: String,
        repo: String,
        current: String,
        range: Range<usize>,
    }

    let pinned: Vec<Pinned> = cmake
        .cpm_calls()
        .into_iter()
        .filter_map(|c| {
            let u = c.uri?;
            let v = u.version?;
            let r = u.version_range?;
            Some(Pinned {
                owner: u.owner,
                repo: u.repo,
                current: v,
                range: r,
            })
        })
        .collect();

    if pinned.is_empty() {
        println!("No pinned CPMAddPackage URIs found in {}.", path.display());
        return Ok(());
    }

    println!("Checking {} package(s)...", pinned.len());
    let octo = octocrab::instance();
    let queries = pinned.iter().map(|p| {
        let octo = octo.clone();
        let owner = p.owner.clone();
        let repo = p.repo.clone();
        async move {
            let release = octo
                .repos(&owner, &repo)
                .releases()
                .get_latest()
                .await
                .with_context(|| format!("query latest for {owner}/{repo}"))?;
            anyhow::Ok(release.tag_name)
        }
    });
    let results = futures::future::join_all(queries).await;

    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    for (p, latest) in pinned.iter().zip(results) {
        let pkg = format!("{}/{}", p.owner, p.repo);
        match latest {
            Ok(latest) => {
                if latest == p.current {
                    continue;
                }
                println!("  {pkg}: {} -> {latest}", p.current);
                edits.push((p.range.clone(), latest));
            }
            Err(e) => eprintln!("  {pkg}: {e:#}"),
        }
    }

    if edits.is_empty() {
        println!("Project CMakeLists.txt is up to date.");
        return Ok(());
    }

    if !yes && !confirm("Apply these updates?").await? {
        println!("Aborted.");
        return Ok(());
    }

    let count = edits.len();
    cmake.splice_many(edits);
    cmake.save()?;
    println!("Updated {count} package(s) in {}.", path.display());
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
        } else if let Some(default) = &project.build_config.default {
            if !project.build_dirs.contains_key(default) {
                return Err(anyhow!(
                    "Configured default build dir '{default}' not found. Known: {:?}",
                    dirs
                ));
            }
            default.clone()
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

// ========== Pkg option command ==========

pub(crate) async fn exec_pkg_option(name: String, opts: Vec<String>) -> Result<()> {
    let pairs: Vec<(String, String)> = opts
        .iter()
        .map(|s| {
            let (k, v) = s
                .split_once('=')
                .with_context(|| format!("Expected KEY=VALUE, got {s:?}"))?;
            Ok::<_, anyhow::Error>((k.trim().to_string(), v.trim().to_string()))
        })
        .collect::<Result<_>>()?;

    // Resolve `name` against the global index so an alias like `fmt` works.
    let home = std::env::var("HOME").ok();
    let resolved = home
        .as_ref()
        .and_then(|h| {
            let p = Path::new(h).join(".config/cmk/pkg.json");
            PackageIndex::load_or_create(&p).ok()
        })
        .and_then(|idx| idx.get_pkg_name(&name).ok())
        .unwrap_or_else(|| name.clone());
    let (resolved_owner, resolved_repo) = resolved
        .split_once('/')
        .map(|(o, r)| (o.to_string(), r.to_string()))
        .unwrap_or_else(|| (String::new(), name.clone()));

    let project_root = get_project_root().await?;
    let path = project_root.join("CMakeLists.txt");
    if !path.exists() {
        return Err(anyhow!("CMakeLists.txt not found at {}", path.display()));
    }

    let mut cmake = CMakeFile::parse_path(&path)?;
    let calls = cmake.cpm_calls();

    let target = calls.into_iter().find(|c| {
        c.uri.as_ref().is_some_and(|u| {
            // owner/repo match wins; otherwise fall back to repo basename.
            (!resolved_owner.is_empty()
                && u.owner.eq_ignore_ascii_case(&resolved_owner)
                && u.repo.eq_ignore_ascii_case(&resolved_repo))
                || (resolved_owner.is_empty() && u.repo.eq_ignore_ascii_case(&resolved_repo))
        })
    });

    let target = target.with_context(|| {
        format!(
            "No URI-form CPMAddPackage matching '{name}' found in {}",
            path.display()
        )
    })?;
    let uri = target.uri.as_ref().unwrap();

    let new_text = render_uri_as_keyword(uri, &pairs);
    cmake.splice(target.call_range.clone(), &new_text);
    cmake.save()?;
    println!(
        "Rewrote {}/{} as keyword form with {} option(s) in {}",
        uri.owner,
        uri.repo,
        pairs.len(),
        path.display()
    );
    Ok(())
}

// ========== Init command ==========

const CMK_TOML_TEMPLATE: &str = r#"# cmk project configuration. See https://github.com/zhscn/cmk for the full schema.

# [build]
# # Used when multiple build dirs exist and PWD isn't inside one.
# default = "build/debug"

# [vars]
# DEPS_DIR = "${PROJECT_ROOT}/.deps"
# DEPS_INSTALL = "${DEPS_DIR}/install"

# [env]
# PATH = { prepend = ["${DEPS_INSTALL}/bin"] }

[fmt]
ignore = ["third_party/**", "build/**"]

[lint]
ignore = ["third_party/**", "build/**"]
# warnings_as_errors = false
# header_filter = "^(src|include)/"
# extra_args = ["-quiet"]
"#;

pub(crate) async fn exec_init(force: bool) -> Result<()> {
    let project_root = get_project_root().await?;
    let path = project_root.join(".cmk.toml");
    if path.exists() && !force {
        return Err(anyhow!(
            "{} already exists (pass --force to overwrite)",
            path.display()
        ));
    }
    tokio::fs::write(&path, CMK_TOML_TEMPLATE).await?;
    println!("Wrote {}", path.display());
    Ok(())
}

// ========== Source file selection (shared by fmt/lint) ==========

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

/// Translation units only — excludes headers. Used by lint since headers
/// have no entry in compile_commands.json.
fn is_translation_unit(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(
            "c" | "cc"
                | "cpp"
                | "cxx"
                | "c++"
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

#[derive(Debug, Clone, Copy)]
pub(crate) enum FileSelection {
    All,
    Staged,
    Unstaged,
    Changed,
}

impl FileSelection {
    fn from_flags(all: bool, staged: bool, unstaged: bool) -> Self {
        if all {
            Self::All
        } else if staged {
            Self::Staged
        } else if unstaged {
            Self::Unstaged
        } else {
            Self::Changed
        }
    }
}

async fn run_git(args: &[&str], project_root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(project_root)
        .output()
        .await?;
    Ok(String::from_utf8(output.stdout)?)
}

fn resolve_single_file(name: &str) -> Result<PathBuf> {
    let p = PathBuf::from(name);
    let abs = if p.is_absolute() {
        p
    } else {
        std::env::current_dir()?.join(p)
    };
    if !abs.exists() {
        return Err(anyhow!("File not found: {}", abs.display()));
    }
    Ok(abs)
}

async fn collect_source_files(
    project_root: &Path,
    selection: FileSelection,
    ignore_patterns: &[String],
    verbose: bool,
) -> Result<Vec<PathBuf>> {
    let output_str = match selection {
        FileSelection::All => run_git(&["ls-files"], project_root).await?,
        FileSelection::Staged => {
            run_git(&["diff", "--name-only", "--cached"], project_root).await?
        }
        FileSelection::Unstaged => run_git(&["diff", "--name-only"], project_root).await?,
        FileSelection::Changed => {
            let output = Command::new("git")
                .args(["diff", "--name-only", "HEAD"])
                .current_dir(project_root)
                .output()
                .await?;
            if output.status.success() {
                String::from_utf8(output.stdout)?
            } else {
                run_git(&["diff", "--name-only", "--cached"], project_root).await?
            }
        }
    };

    let candidates: Vec<PathBuf> = output_str
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| project_root.join(line))
        .filter(|path| path.exists())
        .collect();

    let candidates = if ignore_patterns.is_empty() {
        candidates
    } else {
        let mut builder = globset::GlobSetBuilder::new();
        for pattern in ignore_patterns {
            builder.add(globset::Glob::new(pattern)?);
        }
        let ignore_set = builder.build()?;
        candidates
            .into_iter()
            .filter(|path| {
                let rel = path.strip_prefix(project_root).unwrap_or(path);
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

    Ok(candidates
        .into_iter()
        .filter(|path| {
            let is_src = is_c_or_cpp(path);
            if verbose && !is_src {
                println!("Skipping (not C/C++): {}", path.display());
            }
            is_src
        })
        .collect())
}

// ========== Fmt command ==========

pub(crate) async fn exec_fmt(
    file: Option<String>,
    all: bool,
    staged: bool,
    unstaged: bool,
    dry_run: bool,
    verbose: bool,
) -> Result<()> {
    let project_root = get_project_root().await?;
    let fmt_config = FmtConfig::load(&project_root)?;
    let files = if let Some(name) = file {
        vec![resolve_single_file(&name)?]
    } else {
        let selection = FileSelection::from_flags(all, staged, unstaged);
        collect_source_files(&project_root, selection, &fmt_config.ignore, verbose).await?
    };

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

// ========== Lint command ==========

fn read_compile_db_files(cdb_path: &Path) -> Result<Vec<PathBuf>> {
    #[derive(serde::Deserialize)]
    struct Entry {
        file: String,
        #[serde(default)]
        directory: Option<String>,
    }
    let content = std::fs::read_to_string(cdb_path)
        .with_context(|| format!("Failed to read {}", cdb_path.display()))?;
    let entries: Vec<Entry> = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", cdb_path.display()))?;
    let mut files: Vec<PathBuf> = entries
        .into_iter()
        .map(|e| {
            let p = PathBuf::from(&e.file);
            if p.is_absolute() {
                p
            } else if let Some(dir) = e.directory {
                PathBuf::from(dir).join(p)
            } else {
                p
            }
        })
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
}

const LINT_CACHE_VERSION: &str = "v1";

#[derive(serde::Serialize, serde::Deserialize)]
struct LintCacheEntry {
    signature: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
    warnings: usize,
    errors: usize,
}

fn cache_key_for(source: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(source.to_string_lossy().as_bytes());
    format!("{:x}", h.finalize())
}

fn collect_clang_tidy_configs(project_root: &Path, source: &Path) -> Vec<PathBuf> {
    let start = source.parent().unwrap_or(source);
    let mut configs = Vec::new();
    let mut dir = start.to_path_buf();
    loop {
        let cfg = dir.join(".clang-tidy");
        if cfg.exists() {
            configs.push(cfg);
        }
        if dir == project_root {
            break;
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => break,
        }
    }
    configs
}

fn compute_signature(
    source: &Path,
    cdb_path: &Path,
    base_args: &[String],
    project_root: &Path,
) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(LINT_CACHE_VERSION.as_bytes());
    h.update([0u8]);
    h.update(source.to_string_lossy().as_bytes());
    h.update([0u8]);

    let src_meta =
        std::fs::metadata(source).with_context(|| format!("stat {}", source.display()))?;
    h.update(src_meta.len().to_le_bytes());
    let src_mtime = src_meta
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    h.update(src_mtime.as_nanos().to_le_bytes());

    let cdb_meta = std::fs::metadata(cdb_path)?;
    let cdb_mtime = cdb_meta
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    h.update(cdb_mtime.as_nanos().to_le_bytes());

    for arg in base_args {
        h.update(arg.as_bytes());
        h.update([0u8]);
    }

    for cfg in collect_clang_tidy_configs(project_root, source) {
        h.update(cfg.to_string_lossy().as_bytes());
        if let Ok(meta) = std::fs::metadata(&cfg)
            && let Ok(mt) = meta.modified()
        {
            h.update(
                mt.duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    .to_le_bytes(),
            );
        }
    }

    Ok(format!("{:x}", h.finalize()))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn exec_lint(
    build: Option<String>,
    file: Option<String>,
    interactive: bool,
    all: bool,
    staged: bool,
    unstaged: bool,
    fix: bool,
    warnings_as_errors: bool,
    no_cache: bool,
    verbose: bool,
) -> Result<()> {
    let project = CMakeProject::new().await?;
    let project_root = project.project_root.clone();
    let build_dir = project.resolve_build_dir(build.as_deref()).await?.clone();

    let cdb = build_dir.join("compile_commands.json");
    if !cdb.exists() {
        let key = project
            .build_dirs
            .iter()
            .find(|(_, p)| *p == &build_dir)
            .map(|(k, _)| k.clone());
        eprintln!(
            "compile_commands.json missing in {} — running `cmake` to generate it.",
            build_dir.display()
        );
        project.refresh_build_dir(key.as_deref()).await?;
        if !cdb.exists() {
            return Err(anyhow!(
                "compile_commands.json still missing after refresh. Ensure CMAKE_EXPORT_COMPILE_COMMANDS=ON in {}",
                build_dir.display()
            ));
        }
    }

    let lint_config = LintConfig::load(&project_root)?;

    let files: Vec<PathBuf> = if let Some(name) = file {
        vec![resolve_single_file(&name)?]
    } else if interactive {
        let candidates = read_compile_db_files(&cdb)?;
        if candidates.is_empty() {
            return Err(anyhow!("No source files in {}", cdb.display()));
        }
        let display: Vec<String> = candidates
            .iter()
            .map(|p| {
                p.strip_prefix(&project_root)
                    .unwrap_or(p)
                    .display()
                    .to_string()
            })
            .collect();
        let picked = completing_read(&display).await?;
        if picked.is_empty() {
            return Err(anyhow!("No source file selected"));
        }
        let chosen = candidates
            .into_iter()
            .find(|p| {
                p.strip_prefix(&project_root)
                    .unwrap_or(p)
                    .display()
                    .to_string()
                    == picked
            })
            .with_context(|| format!("Selected file '{picked}' not found in compile db"))?;
        vec![chosen]
    } else {
        let selection = FileSelection::from_flags(all, staged, unstaged);
        let collected =
            collect_source_files(&project_root, selection, &lint_config.ignore, verbose).await?;
        collected
            .into_iter()
            .filter(|path| {
                let is_tu = is_translation_unit(path);
                if verbose && !is_tu {
                    println!("Skipping (header): {}", path.display());
                }
                is_tu
            })
            .collect()
    };

    if files.is_empty() {
        println!("No source files to lint.");
        return Ok(());
    }

    if verbose {
        for file in &files {
            println!("{}", file.display());
        }
    }

    let warnings_as_errors = warnings_as_errors || lint_config.warnings_as_errors;
    let header_filter = lint_config.header_filter.clone();
    let extra_args = Arc::new(lint_config.extra_args);
    let cache_enabled = !no_cache && !fix;
    let cache_dir = build_dir.join(".cmk-lint-cache").join(LINT_CACHE_VERSION);
    if cache_enabled {
        std::fs::create_dir_all(&cache_dir)?;
    }
    let cache_dir = Arc::new(cache_dir);
    let cdb = Arc::new(cdb);
    let build_dir = Arc::new(build_dir);
    let project_root = Arc::new(project_root);
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stdout());

    let build_args = move || -> Vec<String> {
        let mut args = vec!["-p".to_string(), build_dir.display().to_string()];
        if let Some(filter) = &header_filter {
            args.push(format!("-header-filter={filter}"));
        }
        if warnings_as_errors {
            args.push("-warnings-as-errors=*".to_string());
        }
        if fix {
            args.push("--fix".to_string());
        }
        if use_color {
            args.push("--use-color".to_string());
        }
        for a in extra_args.iter() {
            args.push(a.clone());
        }
        args
    };

    let total = files.len();
    let jobs = if fix {
        1
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    };
    let chunk_size = total.div_ceil(jobs);

    #[derive(Clone)]
    struct FileReport {
        file: PathBuf,
        warnings: usize,
        errors: usize,
    }

    let failed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let cache_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let reports = Arc::new(tokio::sync::Mutex::new(Vec::<FileReport>::new()));
    let stdout_lock = Arc::new(tokio::sync::Mutex::new(()));
    let mut handles = Vec::new();

    for chunk in files.chunks(chunk_size) {
        let chunk: Vec<PathBuf> = chunk.to_vec();
        let project_root = Arc::clone(&project_root);
        let failed = Arc::clone(&failed);
        let cache_hits = Arc::clone(&cache_hits);
        let reports = Arc::clone(&reports);
        let stdout_lock = Arc::clone(&stdout_lock);
        let cache_dir = Arc::clone(&cache_dir);
        let cdb = Arc::clone(&cdb);
        let base_args = build_args();
        handles.push(tokio::spawn(async move {
            for file in &chunk {
                let signature = if cache_enabled {
                    compute_signature(file, &cdb, &base_args, &project_root).ok()
                } else {
                    None
                };
                let cache_path = signature
                    .as_ref()
                    .map(|_| cache_dir.join(format!("{}.json", cache_key_for(file))));

                let cached: Option<LintCacheEntry> = cache_path.as_ref().and_then(|p| {
                    std::fs::read(p)
                        .ok()
                        .and_then(|b| serde_json::from_slice(&b).ok())
                });

                let (stdout_s, stderr_s, success, warnings, errors, from_cache) =
                    if let (Some(sig), Some(entry)) = (signature.as_ref(), cached.as_ref())
                        && &entry.signature == sig
                    {
                        cache_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        (
                            entry.stdout.clone(),
                            entry.stderr.clone(),
                            entry.exit_code == 0,
                            entry.warnings,
                            entry.errors,
                            true,
                        )
                    } else {
                        let mut cmd = Command::new("clang-tidy");
                        cmd.args(&base_args)
                            .arg(file)
                            .current_dir(project_root.as_ref());
                        let output = match cmd.output().await {
                            Ok(o) => o,
                            Err(e) => {
                                failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let _g = stdout_lock.lock().await;
                                eprintln!("clang-tidy failed to start for {}: {e}", file.display());
                                continue;
                            }
                        };
                        let success = output.status.success();
                        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                        let warnings =
                            count_diag(&stdout, "warning:") + count_diag(&stderr, "warning:");
                        let errors = count_diag(&stdout, "error:") + count_diag(&stderr, "error:");
                        if let (Some(sig), Some(p)) = (signature.as_ref(), cache_path.as_ref()) {
                            let entry = LintCacheEntry {
                                signature: sig.clone(),
                                exit_code: output.status.code().unwrap_or(-1),
                                stdout: stdout.clone(),
                                stderr: stderr.clone(),
                                warnings,
                                errors,
                            };
                            if let Ok(serialized) = serde_json::to_vec(&entry) {
                                let _ = std::fs::write(p, serialized);
                            }
                        }
                        (stdout, stderr, success, warnings, errors, false)
                    };

                if !success {
                    failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                let has_output = !stdout_s.trim().is_empty() || !stderr_s.trim().is_empty();
                if has_output || !success {
                    let _g = stdout_lock.lock().await;
                    let tag = if from_cache { " (cached)" } else { "" };
                    if use_color {
                        println!("\x1b[1;36m── {}{tag} ──\x1b[0m", file.display());
                    } else {
                        println!("── {}{tag} ──", file.display());
                    }
                    if !stdout_s.is_empty() {
                        print!("{stdout_s}");
                    }
                    if !stderr_s.is_empty() {
                        eprint!("{stderr_s}");
                    }
                }
                if warnings > 0 || errors > 0 || !success {
                    reports.lock().await.push(FileReport {
                        file: file.clone(),
                        warnings,
                        errors,
                    });
                }
            }
        }));
    }

    for handle in handles {
        handle.await?;
    }

    let failed = failed.load(std::sync::atomic::Ordering::Relaxed);
    let reports = Arc::into_inner(reports)
        .expect("all tasks joined")
        .into_inner();

    if !reports.is_empty() {
        let bold = if use_color { "\x1b[1m" } else { "" };
        let yellow = if use_color { "\x1b[33m" } else { "" };
        let red = if use_color { "\x1b[31m" } else { "" };
        let reset = if use_color { "\x1b[0m" } else { "" };
        println!("\n{bold}Lint summary:{reset}");
        for r in &reports {
            let path = r.file.display();
            let mut parts = Vec::new();
            if r.warnings > 0 {
                parts.push(format!("{yellow}{} warning(s){reset}", r.warnings));
            }
            if r.errors > 0 {
                parts.push(format!("{red}{} error(s){reset}", r.errors));
            }
            if parts.is_empty() {
                parts.push(format!("{red}failed{reset}"));
            }
            println!("  {path}: {}", parts.join(", "));
        }
    }

    if failed > 0 {
        return Err(anyhow!("{failed}/{total} file(s) failed clang-tidy"));
    }
    let hits = cache_hits.load(std::sync::atomic::Ordering::Relaxed);
    let cache_note = if cache_enabled {
        format!(" ({hits} cached)")
    } else {
        String::new()
    };
    println!(
        "Linted {total} file(s){cache_note}, {} with diagnostics.",
        reports.len()
    );
    Ok(())
}

fn count_diag(text: &str, marker: &str) -> usize {
    text.lines().filter(|l| l.contains(marker)).count()
}
