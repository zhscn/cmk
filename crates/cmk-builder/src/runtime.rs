use std::path::PathBuf;

use crate::container::ContainerRuntime;

/// Find docker or podman in PATH. Prefer docker.
pub fn detect() -> Option<(ContainerRuntime, PathBuf)> {
    if let Some(p) = find_in_path("docker") {
        return Some((ContainerRuntime::Docker, p));
    }
    if let Some(p) = find_in_path("podman") {
        return Some((ContainerRuntime::Podman, p));
    }
    None
}

pub fn locate(rt: ContainerRuntime) -> Option<PathBuf> {
    match rt {
        ContainerRuntime::Docker => find_in_path("docker"),
        ContainerRuntime::Podman => find_in_path("podman"),
    }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for p in std::env::split_paths(&path) {
        let cand = p.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

#[cfg(unix)]
pub fn current_uid_gid() -> (u32, u32) {
    unsafe { (libc::getuid(), libc::getgid()) }
}

#[cfg(not(unix))]
pub fn current_uid_gid() -> (u32, u32) {
    (1000, 1000)
}
