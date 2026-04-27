use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::store::Store;

/// Resolution chain (design §4.2):
/// 1. `$CMK_TOOLCHAIN` env override
/// 2. nearest `.cmk-toolchain` walking from `cwd` up to `/`
/// 3. `~/.cmk/current`
pub fn resolve(store: &Store, cwd: Option<&Path>) -> Result<String> {
    if let Ok(v) = std::env::var("CMK_TOOLCHAIN") {
        let v = v.trim();
        if !v.is_empty() {
            return Ok(v.to_string());
        }
    }
    if let Some(cwd) = cwd
        && let Some(v) = read_clang_version(cwd)?
    {
        return Ok(v);
    }
    if let Some(v) = store.read_current()?
        && !v.is_empty()
    {
        return Ok(v);
    }
    Err(Error::NoVersionSelected)
}

fn read_clang_version(start: &Path) -> Result<Option<String>> {
    let mut cur: Option<PathBuf> = Some(start.to_path_buf());
    while let Some(dir) = cur {
        let cv = dir.join(".cmk-toolchain");
        if cv.is_file() {
            let text = std::fs::read_to_string(&cv)?;
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Ok(Some(trimmed.to_string()));
            }
        }
        cur = dir.parent().map(Path::to_path_buf);
    }
    Ok(None)
}
