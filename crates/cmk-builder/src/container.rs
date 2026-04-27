use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerRuntime {
    Docker,
    Podman,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkPolicy {
    None,
    Host,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mount {
    pub host: PathBuf,
    pub container: PathBuf,
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMapping {
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerSpec {
    pub image: String,
    pub runtime: ContainerRuntime,
    pub mounts: Vec<Mount>,
    pub env: Vec<(String, String)>,
    pub user: UserMapping,
    pub network: NetworkPolicy,
}
