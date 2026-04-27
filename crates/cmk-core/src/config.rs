use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// `~/.cmk/config.toml` schema (design §5.2).
///
/// ```toml
/// registries = [
///   "github:my-org/cmk-dist",
///   "https://mirror.internal.example.com/cmk",
/// ]
/// ```
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub registries: Vec<String>,
}

impl Config {
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }
}
