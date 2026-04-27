use crate::error::{Error, Result};

/// Stringly-typed platform key matching the manifest schema in design §5.1:
/// `linux-x86_64`, `linux-aarch64`, `darwin-arm64`.
pub fn current_platform() -> Result<String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        other => return Err(Error::UnsupportedHost(other.into())),
    };
    let arch = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "arm64",
        ("linux", "x86_64") => "x86_64",
        ("linux", "aarch64") => "aarch64",
        (_, a) => return Err(Error::UnsupportedHost(format!("{os}-{a}"))),
    };
    Ok(format!("{os}-{arch}"))
}

/// Stable id used as the on-disk version directory: `<version>-<platform>`.
pub fn install_id(version: &str, platform: &str) -> String {
    format!("{version}-{platform}")
}
