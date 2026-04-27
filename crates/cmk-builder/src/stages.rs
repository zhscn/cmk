use anyhow::{Result, bail};

use crate::recipe::Recipe;

/// Drive the bootstrap → final → pass2 → package pipeline.
/// Real implementation lands in M3 (macOS) / M5 (Linux).
pub fn run(_recipe: &Recipe) -> Result<()> {
    bail!("builder execution lands in M3+; see cmk-builder::recipe types")
}
