use std::path::Path;

use anyhow::{Context, Result, bail};

// Using ABI V9 (the latest): covers basic filesystem rights plus
// truncate, ioctl_dev, and the newer network access controls. Both BindTcp
// and ConnectTcp are handled with no grant rules, so landlock denies all TCP
// binding (the listener is bound before sandboxing) and all outbound connects
// (the server makes no external calls).
//
// Abstract UNIX sockets and cross-process signals are scoped to the sandbox
// domain: the blog neither connects to UNIX sockets nor signals other
// processes, so both are confined against lateral movement within the pod.
#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    use landlock::{
        ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, LandlockStatus, PathBeneath,
        PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus, Scope,
    };

    pub fn apply(static_dir: &Path) -> Result<()> {
        let abi = ABI::V9;
        let access_all = AccessFs::from_all(abi);
        let access_read = AccessFs::from_read(abi);

        let mut ruleset = Ruleset::default()
            .handle_access(access_all)?
            .handle_access(AccessNet::BindTcp)?
            .handle_access(AccessNet::ConnectTcp)?
            .scope(Scope::AbstractUnixSocket | Scope::Signal)?
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
                println!(
                    "Landlock sandbox enforced (effective ABI V{})",
                    *effective_abi as u32
                );
            }
            _ => {
                eprintln!("Warning: Landlock not supported by kernel; running without sandbox");
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
    eprintln!("Warning: Landlock is only available on Linux; running without sandbox");
    Ok(())
}
