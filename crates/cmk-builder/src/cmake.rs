use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

pub fn run(cmd: &mut Command, label: &str) -> Result<()> {
    eprintln!("==> {label}: {cmd:?}");
    let status = cmd
        .status()
        .with_context(|| format!("spawn {label}: {cmd:?}"))?;
    if !status.success() {
        bail!("{label} failed: exit {status}");
    }
    Ok(())
}

pub fn cmake_configure(
    build_dir: &Path,
    source_dir: &Path,
    args: &[String],
    env: &[(&str, &str)],
) -> Result<()> {
    std::fs::create_dir_all(build_dir)?;
    let mut cmd = Command::new("cmake");
    cmd.current_dir(build_dir).arg("-G").arg("Ninja").arg(source_dir);
    for a in args {
        cmd.arg(a);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    run(&mut cmd, "cmake configure")
}

pub fn ninja(build_dir: &Path, target: Option<&str>, jobs: usize) -> Result<()> {
    let mut cmd = Command::new("ninja");
    cmd.current_dir(build_dir).arg("-j").arg(jobs.to_string());
    if let Some(t) = target {
        cmd.arg(t);
    }
    run(&mut cmd, "ninja")
}

pub fn ninja_install(build_dir: &Path) -> Result<()> {
    let mut cmd = Command::new("ninja");
    cmd.current_dir(build_dir).arg("install");
    run(&mut cmd, "ninja install")
}

pub fn detect_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}
