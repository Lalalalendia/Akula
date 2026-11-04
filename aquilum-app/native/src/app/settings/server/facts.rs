//! Typed SSH host facts; sudo listing is not production install authority.
use aquilum_core::server_bootstrap_lab::{
    MachineArch, ReadOnlyHostFacts, SupportedOs,
};
use crate::ui::settings::schema::ServerProbeView;

pub(super) fn classify(facts: ReadOnlyHostFacts) -> ServerProbeView {
    let os = match facts.os {
        SupportedOs::Ubuntu => "Ubuntu",
        SupportedOs::Debian => "Debian",
        SupportedOs::Other => "Other",
    };
    let arch = match facts.arch {
        MachineArch::X86_64 => "x86_64",
        MachineArch::Aarch64 => "aarch64",
        MachineArch::Other => "other",
    };
    let summary = format!(
        "{os} / {arch}; {} CPU; {} MiB RAM; {} MiB free disk; systemd {}; sudo listing {}",
        facts.cpu_count, facts.ram_mib, facts.free_disk_mib,
        if facts.has_systemd { "yes" } else { "no" },
        if facts.has_sudo_rights { "yes" } else { "no" },
    );
    let compatible = matches!(facts.os, SupportedOs::Ubuntu | SupportedOs::Debian)
        && facts.arch == MachineArch::X86_64
        && facts.has_systemd
        && facts.cpu_count > 0
        && facts.ram_mib >= 512
        && facts.free_disk_mib >= 1024;
    if !compatible {
        ServerProbeView::Unsupported { facts: summary }
    } else if !facts.has_sudo_rights {
        ServerProbeView::NeedsAdminAuthorization { facts: summary }
    } else {
        // A sudo listing cannot grant the fixed root-helper transaction.
        ServerProbeView::Observed { facts: summary }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(has_sudo_rights: bool) -> ReadOnlyHostFacts {
        ReadOnlyHostFacts {
            os: SupportedOs::Ubuntu,
            arch: MachineArch::X86_64,
            has_systemd: true,
            has_sudo_rights,
            cpu_count: 2,
            ram_mib: 2048,
            free_disk_mib: 4096,
        }
    }

    #[test]
    fn clean_vps_without_sudo_is_admin_hold_not_unsupported() {
        assert!(matches!(
            classify(host(false)),
            ServerProbeView::NeedsAdminAuthorization { .. }
        ));
        assert!(matches!(classify(host(true)), ServerProbeView::Observed { .. }));
    }

    #[test]
    fn incompatible_platform_is_unsupported_even_with_sudo_listing() {
        let mut bad = host(true);
        bad.arch = MachineArch::Aarch64;
        assert!(matches!(classify(bad), ServerProbeView::Unsupported { .. }));
    }
}
