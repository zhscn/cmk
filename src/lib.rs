use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    cmp::min,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

pub mod default;

pub struct CMakeProject {
    pub project_root: PathBuf,
    pub build_root: PathBuf,
}

impl CMakeProject {
    pub fn new() -> Result<Self> {
        let output = Command::new("git")
            .args([
                "rev-parse",
                "--show-superproject-working-tree",
                "--show-toplevel",
            ])
            .output()?;
        let output = String::from_utf8(output.stdout)?;
        let head = output
            .split("\n")
            .next()
            .with_context(|| "No git repository found")?;
        let project_root = PathBuf::from(head);
        let mut build_root = None;
        for entry in std::fs::read_dir(&project_root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let path = entry.path();
                if path.join("CMakeCache.txt").exists() {
                    build_root = Some(path);
                }
            }
        }
        let build_root = build_root.with_context(|| "No CMake build directory found")?;
        Ok(Self {
            project_root,
            build_root,
        })
    }

    fn prepare_cmake_file_api(&self) -> Result<()> {
        let query_dir = self.build_root.join(".cmake/api/v1/query");
        std::fs::create_dir_all(&query_dir)?;
        let codemodel_file = query_dir.join("codemodel-v2");
        if !codemodel_file.try_exists()? {
            std::fs::File::create(&codemodel_file)?;
        }
        Ok(())
    }

    pub fn refresh_build_dir(&self) -> Result<()> {
        Command::new("cmake")
            .args([
                "-S",
                &self.project_root.to_string_lossy(),
                "-B",
                &self.build_root.to_string_lossy(),
            ])
            .output()?;
        Ok(())
    }

    fn collect_target_reply(&self) -> Result<Vec<String>> {
        let reply_dir = self.build_root.join(".cmake/api/v1/reply");
        if !reply_dir.try_exists()? {
            self.prepare_cmake_file_api()?;
            self.refresh_build_dir()?;
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

    pub fn collect_executable_targets(&self) -> Result<Vec<Target>> {
        let reply = self.collect_target_reply()?;
        let mut targets = Vec::new();
        for reply in reply {
            let path = self.build_root.join(".cmake/api/v1/reply/").join(&reply);
            let content = std::fs::read_to_string(path)?;
            let target = serde_json::from_str::<Target>(&content)?;
            if target.is_executable() && target.artifacts.is_some() {
                targets.push(target);
            }
        }
        Ok(targets)
    }

    pub fn build_target(&self, target: &str) -> Result<()> {
        Command::new("cmake")
            .args([
                "--build",
                &self.build_root.to_string_lossy(),
                "--target",
                target,
            ])
            .spawn()?
            .wait()?;
        Ok(())
    }

    pub fn run_target(&self, target: &Target, args: &[String]) -> Result<()> {
        self.build_target(&target.name)?;
        let path = self
            .build_root
            .join(&target.artifacts.as_ref().unwrap()[0].path);
        Command::new(path).args(args).spawn()?.wait()?;
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
    let mut output = fzf.wait_with_output()?.stdout;
    output.pop();
    Ok(String::from_utf8(output)?)
}
