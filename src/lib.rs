use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::{
    cmp::min,
    collections::HashMap,
    fmt::{self, Display},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tokio::task::JoinHandle;

pub mod default;

pub struct CMakeProject {
    pub project_root: PathBuf,
    pub build_dirs: HashMap<String, PathBuf>,
}

impl CMakeProject {
    pub fn new() -> Result<Self> {
        let max_depth = std::env::var("CMK_MAX_DEPTH")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(2);
        Self::new_with_max_depth(max_depth)
    }

    fn new_with_max_depth(max_depth: usize) -> Result<Self> {
        let output = Command::new("git")
            .args([
                "rev-parse",
                "--show-superproject-working-tree",
                "--show-toplevel",
            ])
            .env("GIT_DISCOVERY_ACROSS_FILESYSTEM", "1")
            .output()?;
        let output = String::from_utf8(output.stdout)?;
        let head = output
            .split("\n")
            .next()
            .with_context(|| "No git repository found")?;
        let project_root = PathBuf::from(head);
        let mut build_dirs = HashMap::new();

        Self::collect_build_dirs(&project_root, &project_root, &mut build_dirs, 1, max_depth)?;

        if build_dirs.is_empty() {
            return Err(anyhow!("No CMake build directories found"));
        }

        Ok(Self {
            project_root,
            build_dirs,
        })
    }

    fn collect_build_dirs(
        project_root: &Path,
        current_dir: &Path,
        build_dirs: &mut HashMap<String, PathBuf>,
        current_depth: usize,
        max_depth: usize,
    ) -> Result<()> {
        for entry in std::fs::read_dir(current_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let path = entry.path();

                if path.join("CMakeCache.txt").try_exists()? {
                    let relative_path = path
                        .strip_prefix(project_root)
                        .unwrap()
                        .to_string_lossy()
                        .to_string();
                    build_dirs.insert(relative_path, path.clone());
                }

                if current_depth < max_depth {
                    Self::collect_build_dirs(
                        project_root,
                        &path,
                        build_dirs,
                        current_depth + 1,
                        max_depth,
                    )?;
                }
            }
        }

        Ok(())
    }

    pub fn get_build_dir(&self, build_dir_name: &str) -> Result<&PathBuf> {
        self.build_dirs
            .get(build_dir_name)
            .with_context(|| format!("Build directory '{build_dir_name}' not found"))
    }

    fn detect_pwd(&self) -> Option<&PathBuf> {
        let pwd = std::env::current_dir().ok()?;
        self.build_dirs
            .values()
            .find(|&path| path == &pwd || pwd.starts_with(path))
    }

    pub fn detect_pwd_key(&self) -> Option<String> {
        let pwd = std::env::current_dir().ok()?;
        self.build_dirs
            .iter()
            .find(|(_, path)| path == &&pwd || pwd.starts_with(path))
            .map(|(key, _)| key.clone())
    }

    pub fn get_build_dir_from_input(&self) -> Result<&PathBuf> {
        if self.build_dirs.len() == 1 {
            self.build_dirs
                .values()
                .next()
                .with_context(|| "No build directories available")
        } else if let Some(p) = self.detect_pwd() {
            Ok(p)
        } else {
            Ok(&self.build_dirs[&completing_read(&self.list_build_dirs())?])
        }
    }

    pub fn list_build_dirs(&self) -> Vec<String> {
        self.build_dirs.keys().cloned().collect()
    }

    fn prepare_cmake_file_api(&self, build_dir: &Path) -> Result<()> {
        let query_dir = build_dir.join(".cmake/api/v1/query");
        std::fs::create_dir_all(&query_dir)?;
        let codemodel_file = query_dir.join("codemodel-v2");
        if !codemodel_file.try_exists()? {
            std::fs::File::create(&codemodel_file)?;
        }
        Ok(())
    }

    pub fn refresh_build_dir(&self, build_dir_name: Option<&str>) -> Result<()> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input()?,
        };

        Command::new("cmake")
            .args([
                "-S",
                &self.project_root.to_string_lossy(),
                "-B",
                &build_dir.to_string_lossy(),
            ])
            .output()?;
        Ok(())
    }

    fn collect_target_reply(&self, build_dir_name: Option<&str>) -> Result<Vec<String>> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input()?,
        };

        let reply_dir = build_dir.join(".cmake/api/v1/reply");
        if !reply_dir.try_exists()? {
            self.prepare_cmake_file_api(build_dir)?;
            self.refresh_build_dir(build_dir_name)?;
        }
        let mut reply = Vec::new();
        for entry in std::fs::read_dir(&reply_dir)? {
            let entry = entry?;
            let filename = entry.file_name();
            let filename = filename.to_string_lossy();
            if filename.starts_with("target-") {
                reply.push(filename.to_string());
            }
        }
        Ok(reply)
    }

    pub fn collect_executable_targets(&self, build_dir_name: Option<&str>) -> Result<Vec<Target>> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input()?,
        };

        let reply = self.collect_target_reply(build_dir_name)?;
        let mut targets = Vec::new();
        for reply in reply {
            let path = build_dir.join(".cmake/api/v1/reply/").join(&reply);
            let content = std::fs::read_to_string(path)?;
            let target = serde_json::from_str::<Target>(&content)?;
            if target.is_executable() && target.artifacts.is_some() {
                targets.push(target);
            }
        }
        Ok(targets)
    }

    pub fn build_target(
        &self,
        target: &str,
        build_dir_name: Option<&str>,
        jobs: usize,
    ) -> Result<()> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input()?,
        };

        let ret = Command::new("cmake")
            .args([
                "--build",
                &build_dir.to_string_lossy(),
                "--target",
                target,
                "-j",
                &jobs.to_string(),
            ])
            .spawn()?
            .wait()?;
        if !ret.success() {
            return Err(anyhow!("{}", ret));
        }
        Ok(())
    }

    fn build_target_silent(&self, target: &str, build_dir_name: Option<&str>) -> Result<()> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input()?,
        };

        let ret = Command::new("cmake")
            .args(["--build", &build_dir.to_string_lossy(), "--target", target])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?
            .wait()?;
        if !ret.success() {
            return Err(anyhow!("{}", ret));
        }
        Ok(())
    }

    pub fn run_target(
        &self,
        target: &Target,
        args: &[String],
        build_dir_name: Option<&str>,
    ) -> Result<()> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input()?,
        };

        self.build_target_silent(&target.name, build_dir_name)?;
        let path = build_dir.join(&target.artifacts.as_ref().unwrap()[0].path);
        let ret = Command::new(path).args(args).spawn()?.wait()?;
        if !ret.success() {
            return Err(anyhow!("{}", ret));
        }
        Ok(())
    }

    pub fn list_all_translation_units(&self, build_dir_name: Option<&str>) -> Result<Vec<String>> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input()?,
        };

        let ninja = Command::new("ninja")
            .args(["-C", &build_dir.to_string_lossy(), "-t", "targets", "all"])
            .stdout(Stdio::piped())
            .spawn()?;
        let output = ninja.wait_with_output()?;
        let output = String::from_utf8(output.stdout)?;
        Ok(output
            .split('\n')
            .filter(|line| line.contains(".o: "))
            .map(|line| line.split(": ").next().unwrap().to_string())
            .collect())
    }

    pub fn build_tu(&self, tu: &str, build_dir_name: Option<&str>) -> Result<()> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input()?,
        };

        let ret = Command::new("ninja")
            .args(["-C", &build_dir.to_string_lossy(), tu])
            .spawn()?
            .wait()?;
        if !ret.success() {
            return Err(anyhow!("{}", ret));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CompDBEntry {
    pub directory: String,
    pub command: String,
    pub file: String,
    pub output: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TargetArtifact {
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Target {
    pub name: String,
    #[serde(rename = "type")]
    pub target_type: String,
    pub artifacts: Option<Vec<TargetArtifact>>,
}

impl Target {
    pub fn is_executable(&self) -> bool {
        self.target_type == "EXECUTABLE"
    }
}

pub fn completing_read(elements: &[String]) -> Result<String> {
    let height = min(elements.len(), 10) + 2;
    let mut fzf = Command::new("fzf")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .args(["--height", &height.to_string()])
        .spawn()?;
    let mut child_stdin = fzf.stdin.take().unwrap();
    for element in elements {
        child_stdin.write_all(element.as_bytes())?;
        child_stdin.write_all(b"\n")?;
    }
    drop(child_stdin);
    let mut output = fzf.wait_with_output()?.stdout;
    if output.ends_with(b"\n") {
        output.pop();
    }
    Ok(String::from_utf8(output)?)
}

#[derive(Debug, Serialize, Deserialize, Hash, Clone)]
pub struct Package {
    pub owner: String,
    pub repo: String,
}

impl Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageIndex {
    pub aliases: HashMap<String, Package>,
    pub releases: HashMap<String, String>,
}

impl PackageIndex {
    pub fn load_or_create(path: &PathBuf) -> Result<Self> {
        if !path.try_exists()? {
            let index = Self {
                aliases: HashMap::new(),
                releases: HashMap::new(),
            };
            index.save(path)?;
            return Ok(index);
        }
        let content = std::fs::read_to_string(path)?;
        let index: PackageIndex = serde_json::from_str(&content)?;
        Ok(index)
    }

    pub fn save(&self, path: &PathBuf) -> Result<()> {
        let content = serde_json::to_string(self)?;
        let parent = path
            .parent()
            .with_context(|| "Failed to get parent directory")?;
        if !parent.try_exists()? {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn get_pkg_name(&self, name: &str) -> Result<String> {
        let pkg_name = if name.contains('/') {
            Some(name.to_string())
        } else {
            self.aliases.get(name).map(|s| s.to_string())
        };
        let pkg_name = pkg_name.with_context(|| format!("Package alias {name} not found"))?;
        Ok(pkg_name)
    }

    pub fn get_release(&self, name: &str) -> Result<&str> {
        let name = self.get_pkg_name(name)?;
        let release = self.releases.get(&name).map(|s| s.as_str());
        release.with_context(|| format!("Release {name} not found"))
    }

    pub async fn add_repo(&mut self, owner: &str, repo: &str) -> Result<()> {
        let octocrab = octocrab::instance();
        let release = octocrab.repos(owner, repo).releases().get_latest().await?;
        let pkg_name = format!("{owner}/{repo}");
        self.aliases.insert(
            repo.to_string(),
            Package {
                owner: owner.to_string(),
                repo: repo.to_string(),
            },
        );
        println!("{}: {}", pkg_name, release.tag_name);
        self.releases.insert(pkg_name, release.tag_name);
        Ok(())
    }

    pub async fn update(&mut self) -> Result<()> {
        let octocrab = octocrab::instance();

        let mut futures = Vec::new();
        for pkg in self.aliases.values() {
            let octocrab = octocrab.clone();
            let pkg = pkg.clone();

            let future: JoinHandle<Result<(String, String)>> = tokio::spawn(async move {
                let release = octocrab
                    .repos(&pkg.owner, &pkg.repo)
                    .releases()
                    .get_latest()
                    .await?;
                Ok((pkg.to_string(), release.tag_name))
            });

            futures.push(future);
        }

        for result in join_all(futures).await {
            match result? {
                Ok((pkg_name, tag_name)) => {
                    let existing = self
                        .releases
                        .get(&pkg_name)
                        .with_context(|| format!("Package {pkg_name} not found"))?;
                    if existing == &tag_name {
                        continue;
                    }
                    println!("{pkg_name}: {existing} -> {tag_name}");
                    self.releases.insert(pkg_name, tag_name);
                }
                Err(e) => {
                    eprintln!("Failed to update package: {e}");
                }
            }
        }

        Ok(())
    }
}
