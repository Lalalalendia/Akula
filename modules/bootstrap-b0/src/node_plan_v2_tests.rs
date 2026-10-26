//! B0 v2/v3 preservation and fail-closed tests.
use super::*;

    struct StaticKex {
        wire: Vec<u8>,
        listed_sudo: bool,
    }
    impl PreviewTransport for StaticKex {
        fn handshake_public_key(&mut self, _: &ServerTarget) -> Result<Vec<u8>, B0Error> {
            Ok(self.wire.clone())
        }
        fn inspect_host(&mut self) -> Result<ReadOnlyHostFacts, B0Error> {
            Ok(ReadOnlyHostFacts {
                os: SupportedOs::Ubuntu,
                arch: MachineArch::X86_64,
                has_systemd: true,
                has_sudo_rights: self.listed_sudo,
                cpu_count: 4,
                ram_mib: 4096,
                free_disk_mib: 8192,
            })
        }
    }

    #[test]
    fn node_v2_discloses_public_udp_and_separates_legacy_plan_digest() {
        let wire = vec![0x91; 32];
        let input = B0Input {
            install_id: Uuid::from_bytes([1; 16]),
            target: ServerTarget {
                host: "vps.example.org".into(),
                ssh_port: 22,
                ssh_user: "ubuntu".into(),
                trusted_key_sha256: digest::digest(&digest::SHA256, &wire)
                    .as_ref().try_into().unwrap(),
            },
            package: PackagePin {
                version: "0.3.13".into(),
                package_sha256: [0x44; 32],
                trusted_signer_id: "test-release-anchor".into(),
            },
        };
        let old = preview(&input, &mut StaticKex { wire: wire.clone(), listed_sudo: true }, || false).unwrap();
        let v2 = preview_node_v2(&input, &mut StaticKex { wire, listed_sudo: true }, || false).unwrap();
        assert!(old.verify_unchanged() && v2.verify_unchanged());
        assert_eq!((old.schema_version, v2.schema_version), (1, 2));
        assert_ne!(old.plan_blake3, v2.plan_blake3);
        assert_eq!(v2.proposed_changes.as_slice(), NODE_V2_ACTIONS.as_slice());
        assert!(v2.proposed_changes.contains(
            &ProposedChange::BindPublicUdp43871WithoutChangingFirewall
        ));
        assert!(!v2.proposed_changes.contains(&ProposedChange::EnableLocalLoopbackOnly));
        assert!(old.proposed_changes.contains(&ProposedChange::EnableLocalLoopbackOnly));
        for index in 0..v2.proposed_changes.len() {
            let mut changed = v2.clone();
            changed.proposed_changes.remove(index);
            changed.plan_blake3 = changed.computed_digest();
            assert!(!changed.verify_unchanged(), "missing action {index}");
        }
        let mut downgrade = v2.clone();
        downgrade.schema_version = 1;
        downgrade.plan_blake3 = downgrade.computed_digest();
        assert!(!downgrade.verify_unchanged());
        let mut wrong_key = v2;
        wrong_key.target.trusted_key_sha256[0] ^= 1;
        wrong_key.plan_blake3 = wrong_key.computed_digest();
        // A changed target can be internally consistent but is no longer the
        // originally reviewed plan. Detached owner signatures bind its digest.
        assert_ne!(wrong_key.plan_blake3, old.plan_blake3);
    }

    #[test]
    fn clean_vps_without_sudo_is_inventory_only_not_installable() {
        let wire = vec![0xa5; 32];
        let input = B0Input {
            install_id: Uuid::from_bytes([2; 16]),
            target: ServerTarget {
                host: "clean-vps.example.org".into(),
                ssh_port: 22,
                ssh_user: "ubuntu".into(),
                trusted_key_sha256: digest::digest(&digest::SHA256, &wire)
                    .as_ref().try_into().unwrap(),
            },
            package: PackagePin {
                version: "0.3.13".into(),
                package_sha256: [0x44; 32],
                trusted_signer_id: "test-release-anchor".into(),
            },
        };
        let mut old_kex = StaticKex { wire: wire.clone(), listed_sudo: false };
        assert_eq!(preview(&input, &mut old_kex, || false), Err(B0Error::MissingPrivilege));
        let mut v2_kex = StaticKex { wire, listed_sudo: false };
        let plan = preview_node_v2(&input, &mut v2_kex, || false).unwrap();
        assert_eq!(plan.schema_version, 2);
        assert!(!plan.inspected_host.has_sudo_rights);
        assert!(plan.verify_unchanged());
        assert!(plan.proposed_changes.contains(
            &ProposedChange::RequireIndependentlyPinnedRootBootstrap
        ));
    }

    #[test]
    fn v3_binds_distinct_signed_helper_without_changing_v2_hash() {
        let wire = vec![0x92; 32];
        let input = B0Input {
            install_id: Uuid::from_bytes([3; 16]),
            target: ServerTarget {
                host: "vps.example.org".into(),
                ssh_port: 22,
                ssh_user: "ubuntu".into(),
                trusted_key_sha256: digest::digest(&digest::SHA256, &wire)
                    .as_ref().try_into().unwrap(),
            },
            package: PackagePin {
                version: "0.3.13".into(),
                package_sha256: [0x44; 32],
                trusted_signer_id: "test-release-anchor".into(),
            },
        };
        let v2 = preview_node_v2(&input, &mut StaticKex {
            wire: wire.clone(), listed_sudo: true,
        }, || false).unwrap();
        let v3 = preview_node_v3(&input, [0x55; 32], &mut StaticKex {
            wire, listed_sudo: true,
        }, || false).unwrap();
        assert!(v2.verify_unchanged() && v3.verify_unchanged());
        assert_eq!(v2.bootstrap_helper_sha256, None);
        assert_eq!(v2.bootstrap_helper_install_path, None);
        assert_eq!(v3.bootstrap_helper_sha256, Some([0x55; 32]));
        assert_eq!(
            v3.bootstrap_helper_install_path.as_deref(),
            Some(ROOT_HELPER_EXECUTABLE_PATH)
        );
        assert!(v3.proposed_changes.contains(
            &ProposedChange::InstallRootOwnedBootstrapHelperAtFixedPath
        ));
        // Old v1/v2 serialized plans and hash domains are untouched; only
        // the opt-in unreleased v3 gains this approval-bound fixed path.
        assert!(!serde_json::to_string(&v2).unwrap().contains("bootstrap_helper_install_path"));
        assert!(serde_json::to_string(&v3).unwrap().contains(ROOT_HELPER_EXECUTABLE_PATH));
        assert_ne!(v2.plan_blake3, v3.plan_blake3);
        assert!(v3.proposed_changes.contains(
            &ProposedChange::VerifySignedBootstrapHelperBeforeRootExecution
        ));
        let mut omitted = v3.clone();
        omitted.bootstrap_helper_sha256 = None;
        omitted.plan_blake3 = omitted.computed_digest();
        assert!(!omitted.verify_unchanged());
        let mut missing_path = v3.clone();
        missing_path.bootstrap_helper_install_path = None;
        missing_path.plan_blake3 = missing_path.computed_digest();
        assert!(!missing_path.verify_unchanged());
        let mut missing_action = v3.clone();
        missing_action.proposed_changes.remove(3);
        missing_action.plan_blake3 = missing_action.computed_digest();
        assert!(!missing_action.verify_unchanged());
        for path in ["/tmp/helper", "/usr/libexec/akula/../akula-root-b1d0-helper",
                     "/usr/libexec/akula/akula-root-b1d0-helper/", "/usr/libexec/akula/other"] {
            let mut altered = v3.clone();
            altered.bootstrap_helper_install_path = Some(path.to_owned());
            altered.plan_blake3 = altered.computed_digest();
            assert!(!altered.verify_unchanged(), "changed root destination {path}");
        }
        // A canonical but old laboratory v3 plan without the newly required
        // destination must never be granted root install authority.
        let mut legacy_v3 = serde_json::to_value(&v3).unwrap();
        legacy_v3.as_object_mut().unwrap().remove("bootstrap_helper_install_path");
        let legacy: DryRunPlan = serde_json::from_value(legacy_v3).unwrap();
        assert!(!legacy.verify_unchanged());
        let mut downgrade = v3.clone();
        downgrade.schema_version = 2;
        downgrade.plan_blake3 = downgrade.computed_digest();
        assert!(!downgrade.verify_unchanged());
        let mut changed = v3.clone();
        changed.bootstrap_helper_sha256 = Some([0x66; 32]);
        changed.plan_blake3 = changed.computed_digest();
        assert_ne!(v3.plan_blake3, changed.plan_blake3);
        // A changed helper might be internally valid, but the old detached
        // owner signature over v3's original plan digest cannot match it.
        assert!(changed.verify_unchanged());
        assert_eq!(
            preview_node_v3(&input, [0; 32], &mut StaticKex {
                wire: vec![], listed_sudo: true,
            }, || false),
            Err(B0Error::InvalidPackage),
        );
        assert_eq!(
            preview_node_v3(&input, [0x44; 32], &mut StaticKex {
                wire: vec![], listed_sudo: true,
            }, || false),
            Err(B0Error::InvalidPackage),
        );
    }
