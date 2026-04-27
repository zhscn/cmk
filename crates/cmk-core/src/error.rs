use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("toml deserialize: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("toml serialize: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("manifest references unknown platform `{0}` (host has no entry)")]
    PlatformMissing(String),
    #[error("package `{0}` not found in manifest for platform `{1}`")]
    PackageMissing(String, String),
    #[error("sha256 mismatch for {path}: got {got}, want {want}")]
    Sha256Mismatch {
        path: PathBuf,
        got: String,
        want: String,
    },
    #[error("version `{0}` is not installed")]
    VersionNotInstalled(String),
    #[error("no version selected (set `cmk toolchain use <ver>`, $CMK_TOOLCHAIN, or .cmk-toolchain)")]
    NoVersionSelected,
    #[error("unsupported host platform: {0}")]
    UnsupportedHost(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
