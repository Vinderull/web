use anyhow::{Result, bail};

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
        ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, LandlockStatus, Ruleset,
        RulesetAttr, RulesetStatus, Scope,
    };

    /// The newest Landlock ABI the sandbox requests (V9): basic filesystem rights
    /// plus truncate, ioctl_dev, the network access controls, and the scope
    /// domains. Kernels that only support an older ABI enforce a downgraded
    /// subset of these protections.
    const REQUESTED_ABI: ABI = ABI::V9;

    pub fn apply() -> Result<()> {
        let abi = REQUESTED_ABI;
        let access_all = AccessFs::from_all(abi);

        let ruleset = Ruleset::default()
            .handle_access(access_all)?
            .handle_access(AccessNet::BindTcp)?
            .handle_access(AccessNet::ConnectTcp)?
            .scope(Scope::AbstractUnixSocket | Scope::Signal)?
            .create()?;

        let status = ruleset
            .set_compatibility(CompatLevel::BestEffort)
            .restrict_self()?;

        match &status.landlock {
            LandlockStatus::Available { effective_abi, .. } => {
                if status.ruleset == RulesetStatus::NotEnforced {
                    bail!("Landlock ruleset could not be enforced despite kernel support");
                }
                if let Some(warning) = downgrade_warning(*effective_abi) {
                    eprintln!("{}", warning);
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

    /// Compose the stderr warning shown when the kernel enforces an ABI lower
    /// than the requested [`REQUESTED_ABI`], or `None` on full enforcement.
    ///
    /// Startup continues either way; the warning exists so an operator notices
    /// that the requested protections were downgraded and that newer
    /// filesystem, network, or scope controls may be unavailable.
    fn downgrade_warning(effective_abi: ABI) -> Option<String> {
        if effective_abi >= REQUESTED_ABI {
            return None;
        }
        Some(format!(
            "Warning: Landlock sandbox enforced with effective ABI V{}, but ABI V{} was \
             requested: protections were downgraded, so newer filesystem, network, or \
             scope controls may be unavailable",
            effective_abi as u32, REQUESTED_ABI as u32,
        ))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn full_v9_enforcement_warns_nothing() {
            assert_eq!(downgrade_warning(ABI::V9), None);
        }

        #[test]
        fn downgraded_abi_warns_with_versions_and_consequences() {
            let warning = downgrade_warning(ABI::V6).expect("downgrade must warn");
            assert!(
                warning.contains("ABI V6"),
                "reports the effective ABI: {warning}"
            );
            assert!(
                warning.contains("ABI V9"),
                "reports the requested ABI: {warning}"
            );
            assert!(
                warning.contains("downgraded"),
                "labels the downgrade: {warning}"
            );
            assert!(
                warning.contains("filesystem"),
                "mentions filesystem controls: {warning}"
            );
            assert!(
                warning.contains("network"),
                "mentions network controls: {warning}"
            );
            assert!(
                warning.contains("scope"),
                "mentions scope controls: {warning}"
            );
        }

        #[test]
        fn every_lower_abi_warns() {
            for abi in [
                ABI::V1,
                ABI::V2,
                ABI::V3,
                ABI::V4,
                ABI::V5,
                ABI::V6,
                ABI::V7,
                ABI::V8,
            ] {
                assert!(downgrade_warning(abi).is_some(), "{abi:?} must warn");
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub fn apply() -> Result<()> {
    linux::apply()
}

#[cfg(not(target_os = "linux"))]
pub fn apply() -> Result<()> {
    eprintln!("Warning: Landlock is only available on Linux; running without sandbox");
    Ok(())
}
