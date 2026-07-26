use std::path::Path;

use anyhow::{Context, Result, bail};

// ponytail: ABI V1 covers basic filesystem rights (read/write/execute).
// Bump to V3+ after testing to also restrict truncate, ioctl_dev, etc.
#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    use landlock::{
        ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, LandlockStatus, PathBeneath,
        PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
    };

    pub fn apply(static_dir: &Path) -> Result<()> {
        let abi = ABI::V1;
        let access_all = AccessFs::from_all(abi);
        let access_read = AccessFs::from_read(abi);

        let mut ruleset = Ruleset::default()
            .handle_access(access_all)?
            .handle_access(AccessNet::BindTcp)?
            .handle_access(AccessNet::ConnectTcp)?
            .create()?;

        // Posts are loaded into memory before sandboxing, so only the static
        // asset dir (served on-demand by ServeDir) needs read access.
        if static_dir.exists() {
            ruleset = ruleset.add_rule(PathBeneath::new(
                PathFd::new(static_dir)
                    .with_context(|| format!("opening {}", static_dir.display()))?,
                access_read,
            ))?;
        }

        let status = ruleset
            .set_compatibility(CompatLevel::BestEffort)
            .restrict_self()?;

        match &status.landlock {
            LandlockStatus::Available { effective_abi, .. } => {
                if status.ruleset == RulesetStatus::NotEnforced {
                    bail!("Landlock ruleset could not be enforced despite kernel support");
                }
                tracing::info!(
                    "Landlock sandbox enforced (effective ABI V{})",
                    *effective_abi as u32
                );
            }
            _ => {
                tracing::warn!("Landlock not supported by kernel; running without sandbox");
            }
        }

        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub fn apply(static_dir: &Path) -> Result<()> {
    linux::apply(static_dir)
}

#[cfg(not(target_os = "linux"))]
pub fn apply(_static_dir: &Path) -> Result<()> {
    tracing::warn!("Landlock is only available on Linux; running without sandbox");
    Ok(())
}
