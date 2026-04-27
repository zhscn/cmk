use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::process::Command;

use cmk_config::{BuildConfig, BuildEnv};
use cmk_core::process::{completing_read, wait_with_cancel};

pub async fn get_project_root() -> Result<PathBuf> {
    let output = Command::new("git")
        .args([
            "rev-parse",
            "--show-superproject-working-tree",
            "--show-toplevel",
        ])
        .env("GIT_DISCOVERY_ACROSS_FILESYSTEM", "1")
        .output()
        .await?;
    let output = String::from_utf8(output.stdout)?;
    let head = output
        .split("\n")
        .next()
        .with_context(|| "No git repository found")?;
    Ok(PathBuf::from(head))
}

pub struct CMakeProject {
    pub project_root: PathBuf,
    pub build_dirs: HashMap<String, PathBuf>,
    pub env_config: BuildEnv,
    pub build_config: BuildConfig,
}

impl CMakeProject {
    pub async fn new() -> Result<Self> {
        let max_depth = std::env::var("CMK_MAX_DEPTH")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(2);
        Self::new_with_max_depth(max_depth).await
    }

    async fn new_with_max_depth(max_depth: usize) -> Result<Self> {
        let project_root = get_project_root().await?;
        let mut build_dirs = HashMap::new();

        Self::collect_build_dirs(&project_root, &project_root, &mut build_dirs, 1, max_depth)?;

        if build_dirs.is_empty() {
            return Err(anyhow!("No CMake build directories found"));
        }

        let env_config = BuildEnv::load(&project_root)?;
        let build_config = BuildConfig::load(&project_root)?;

        Ok(Self {
            project_root,
            build_dirs,
            env_config,
            build_config,
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

    pub async fn get_build_dir_from_input(&self) -> Result<&PathBuf> {
        if self.build_dirs.len() == 1 {
            self.build_dirs
                .values()
                .next()
                .with_context(|| "No build directories available")
        } else if let Some(p) = self.detect_pwd() {
            Ok(p)
        } else if let Some(default) = &self.build_config.default {
            self.build_dirs.get(default).with_context(|| {
                format!(
                    "Configured default build dir '{default}' not found. Known: {:?}",
                    self.list_build_dirs()
                )
            })
        } else {
            let res = completing_read(&self.list_build_dirs()).await?;
            if res.is_empty() {
                return Err(anyhow!("No build directory selected"));
            }
            Ok(&self.build_dirs[&res])
        }
    }

    /// Resolve a build dir given an optional explicit name. When `None`,
    /// follows the cascade: single → PWD → configured default → fzf prompt.
    pub async fn resolve_build_dir(&self, name: Option<&str>) -> Result<&PathBuf> {
        match name {
            Some(n) => self.get_build_dir(n),
            None => self.get_build_dir_from_input().await,
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

    pub async fn refresh_build_dir(&self, build_dir_name: Option<&str>) -> Result<()> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input().await?,
        };

        let mut cmd = Command::new("cmake");
        cmd.args([
            "-S",
            &self.project_root.to_string_lossy(),
            "-B",
            &build_dir.to_string_lossy(),
        ]);
        self.env_config
            .apply_to_command(&mut cmd, &self.env_config.build_env(Some(build_dir)));
        cmd.output().await?;
        Ok(())
    }

    async fn collect_target_reply(&self, build_dir_name: Option<&str>) -> Result<Vec<String>> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input().await?,
        };

        let reply_dir = build_dir.join(".cmake/api/v1/reply");
        if !reply_dir.try_exists()? {
            self.prepare_cmake_file_api(build_dir)?;
            self.refresh_build_dir(build_dir_name).await?;
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

    pub async fn collect_executable_targets(
        &self,
        build_dir_name: Option<&str>,
    ) -> Result<Vec<Target>> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input().await?,
        };

        let reply = self.collect_target_reply(build_dir_name).await?;
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

    pub async fn build_target(
        &self,
        target: &str,
        build_dir_name: Option<&str>,
        jobs: usize,
    ) -> Result<()> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input().await?,
        };

        let mut cmd = Command::new("cmake");
        cmd.args([
            "--build",
            &build_dir.to_string_lossy(),
            "--target",
            target,
            "-j",
            &jobs.to_string(),
        ]);
        self.env_config
            .apply_to_command(&mut cmd, &self.env_config.build_env(Some(build_dir)));
        let mut child = cmd.spawn()?;
        let ret = wait_with_cancel(&mut child).await?;
        if !ret.success() {
            return Err(anyhow!("{}", ret));
        }
        Ok(())
    }

    async fn build_target_silent(&self, target: &str, build_dir_name: Option<&str>) -> Result<()> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input().await?,
        };

        let mut cmd = Command::new("cmake");
        cmd.args(["--build", &build_dir.to_string_lossy(), "--target", target])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        self.env_config
            .apply_to_command(&mut cmd, &self.env_config.build_env(Some(build_dir)));
        let mut child = cmd.spawn()?;
        let ret = wait_with_cancel(&mut child).await?;
        if !ret.success() {
            return Err(anyhow!("{}", ret));
        }
        Ok(())
    }

    pub async fn run_target(
        &self,
        target: &Target,
        args: &[String],
        build_dir_name: Option<&str>,
    ) -> Result<()> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input().await?,
        };

        self.build_target_silent(&target.name, build_dir_name)
            .await?;
        let path = build_dir.join(&target.artifacts.as_ref().unwrap()[0].path);
        let mut cmd = Command::new(path);
        cmd.args(args);
        self.env_config.apply_to_command(
            &mut cmd,
            &self.env_config.run_env(Some(&target.name), Some(build_dir)),
        );
        let mut child = cmd.spawn()?;
        let ret = wait_with_cancel(&mut child).await?;
        if !ret.success() {
            return Err(anyhow!("{}", ret));
        }
        Ok(())
    }

    pub async fn list_all_translation_units(
        &self,
        build_dir_name: Option<&str>,
    ) -> Result<Vec<String>> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input().await?,
        };

        let mut cmd = Command::new("ninja");
        cmd.args(["-C", &build_dir.to_string_lossy(), "-t", "targets", "all"])
            .stdout(Stdio::piped());
        self.env_config
            .apply_to_command(&mut cmd, &self.env_config.build_env(Some(build_dir)));
        let output = cmd.output().await?;
        let output = String::from_utf8(output.stdout)?;
        Ok(output
            .split('\n')
            .filter(|line| line.contains(".o: "))
            .map(|line| line.split(": ").next().unwrap().to_string())
            .collect())
    }

    pub async fn build_tu(&self, tu: &str, build_dir_name: Option<&str>) -> Result<()> {
        let build_dir = match build_dir_name {
            Some(name) => self.get_build_dir(name)?,
            None => self.get_build_dir_from_input().await?,
        };

        let mut cmd = Command::new("ninja");
        cmd.args(["-C", &build_dir.to_string_lossy(), tu]);
        self.env_config
            .apply_to_command(&mut cmd, &self.env_config.build_env(Some(build_dir)));
        let mut child = cmd.spawn()?;
        let ret = wait_with_cancel(&mut child).await?;
        if !ret.success() {
            return Err(anyhow!("{}", ret));
        }
        Ok(())
    }
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
