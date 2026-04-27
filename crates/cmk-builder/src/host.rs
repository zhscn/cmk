use anyhow::{Result, bail};

use crate::recipe::{Baseline, HostToolchain, Os, Target};

/// Stub of the host-selection policy from design §7.3. M3+ replaces
/// this with real probing of cmk toolchain store and devtoolset/gcc-toolset.
pub fn pick_host(target: &Target, _baseline: &Baseline) -> Result<HostToolchain> {
    match target.os {
        Os::Macos => bail!("macOS host detection lands in M3"),
        Os::Linux => bail!("Linux host detection lands in M5"),
    }
}
