use super::*;
use crate::server_bootstrap_lab::{
    preview_node_v3, B0Error, B0Input, PackagePin, PreviewTransport, ReadOnlyHostFacts,
    ServerTarget,
};
use crate::server_owner_approval_wire::OwnerFloorWire;
use ring::digest::{digest, SHA256};
use ring::signature::{Ed25519KeyPair, KeyPair};
use uuid::Uuid;

const NOW: u64 = 1_800_000_000;
const NODE_SHA: [u8; 32] = [0x44; 32];
const HELPER_SHA: [u8; 32] = [0x55; 32];
const MANIFEST: [u8; 32] = [0x77; 32];

struct SignedKexFixture(Vec<u8>);
impl PreviewTransport for SignedKexFixture {
    fn handshake_public_key(&mut self, _: &ServerTarget) -> Result<Vec<u8>, B0Error> {
        Ok(self.0.clone())
    }

    fn inspect_host(&mut self) -> Result<ReadOnlyHostFacts, B0Error> {
        Ok(ReadOnlyHostFacts {
            os: SupportedOs::Ubuntu,
            arch: MachineArch::X86_64,
            has_systemd: true,
            has_sudo_rights: true,
            cpu_count: 4,
            ram_mib: 4096,
            free_disk_mib: 8192,
        })
    }
}

fn signing_key() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[0x39; 32]).unwrap()
}

fn signed_claim(claim: &OwnerApprovalWireV1) -> [u8; 64] {
    signing_key()
        .sign(&claim.canonical_bytes())
        .as_ref()
        .try_into()
        .unwrap()
}

fn fixture() -> (
    Vec<u8>,
    OwnerApprovalWireV1,
    [u8; 64],
    RootIndependentPins<'static>,
) {
    let key = b"actual-test-ssh-host-wire-byte-identity".to_vec();
    let ssh_hash: [u8; 32] = digest(&SHA256, &key).as_ref().try_into().unwrap();
    let input = B0Input {
        install_id: Uuid::from_bytes([0x11; 16]),
        target: ServerTarget {
            host: "vps.example.org".into(),
            ssh_port: 22,
            ssh_user: "ubuntu".into(),
            trusted_key_sha256: ssh_hash,
        },
        package: PackagePin {
            version: "0.3.13".into(),
            package_sha256: NODE_SHA,
            trusted_signer_id: "akula-ci-release".into(),
        },
    };
    let plan = preview_node_v3(&input, HELPER_SHA, &mut SignedKexFixture(key), || false).unwrap();
    let plan_hash = blake3::Hash::from_hex(&plan.plan_blake3).unwrap();
    let previous = OwnerFloorWire {
        generation: 0,
        sequence: 0,
        version: [0; 3],
        manifest_fingerprint: [0; 32],
        chain_tip: [0; 32],
    };
    let claim = OwnerApprovalWireV1 {
        approver_id: "akula-ci-owner".into(),
        install_id: *input.install_id.as_bytes(),
        plan_digest: *plan_hash.as_bytes(),
        ssh_host_sha256: ssh_hash,
        manifest_fingerprint: MANIFEST,
        previous,
        proposed: OwnerFloorWire {
            generation: 1,
            sequence: 1,
            version: [0, 3, 13],
            manifest_fingerprint: MANIFEST,
            chain_tip: [0x66; 32],
        },
        staged_package_sha256: NODE_SHA,
        staged_package_size: 4096,
        expected_witness_revision: 0,
        approval_nonce: [0x88; 16],
        issued_at_unix: NOW - 2,
        expires_at_unix: NOW + 40,
    };
    let pins = RootIndependentPins {
        owner_id: "akula-ci-owner",
        owner_ed25519: signing_key().public_key().as_ref().try_into().unwrap(),
        ssh_host_sha256: ssh_hash,
        bootstrap_helper_sha256: HELPER_SHA,
        node_sha256: NODE_SHA,
        node_size: 4096,
        manifest_fingerprint: MANIFEST,
        release_signer_id: "akula-ci-release",
        release_version: "0.3.13",
        host_os: SupportedOs::Ubuntu,
        host_arch: MachineArch::X86_64,
        has_systemd: true,
        witness_revision: 0,
    };
    (
        serde_json::to_vec(&plan).unwrap(),
        claim.clone(),
        signed_claim(&claim),
        pins,
    )
}

#[test]
fn exact_canonical_v3_plan_and_owner_signature_are_inertly_correlated() {
    let (raw, claim, signature, pins) = fixture();
    let scope = verify_root_b0_v3_scope(&raw, &claim, &signature, pins, NOW).unwrap();
    assert_eq!(scope.install_id, [0x11; 16]);
    assert_eq!(scope.plan_digest, claim.plan_digest);
    assert_eq!(scope.node_sha256, NODE_SHA);
    assert_eq!(scope.bootstrap_helper_sha256, HELPER_SHA);
    let parsed: DryRunPlan = serde_json::from_slice(&raw).unwrap();
    assert_eq!(
        parsed.bootstrap_helper_install_path.as_deref(),
        Some(crate::server_bootstrap_lab::ROOT_HELPER_EXECUTABLE_PATH)
    );
    assert_eq!(scope.approval_nonce, [0x88; 16]);
}

#[test]
fn changed_or_duplicate_json_is_not_an_install_plan() {
    let (raw, claim, signature, pins) = fixture();
    let bad = [
        [vec![b' '], raw.clone()].concat(),
        [raw.clone(), vec![b' ']].concat(),
        [
            raw[..raw.len() - 1].to_vec(),
            b",\"unexpected\":true}".to_vec(),
        ]
        .concat(),
        [
            raw[..raw.len() - 1].to_vec(),
            b",\"schema_version\":3}".to_vec(),
        ]
        .concat(),
        vec![b'X'; MAX_CANONICAL_PLAN_BYTES + 1],
    ];
    for (i, case) in bad.iter().enumerate() {
        assert!(
            verify_root_b0_v3_scope(case, &claim, &signature, pins, NOW).is_err(),
            "noncanonical case {i}",
        );
    }
}

#[test]
fn rehashed_downgrade_changed_action_or_helper_still_refuses() {
    let (raw, claim, signature, pins) = fixture();
    for variant in 0..5 {
        let mut plan: DryRunPlan = serde_json::from_slice(&raw).unwrap();
        match variant {
            0 => plan.schema_version = 2,
            1 => {
                plan.proposed_changes.pop();
            }
            2 => {
                plan.bootstrap_helper_sha256 = Some([0x99; 32]);
            }
            3 => plan.bootstrap_helper_install_path = Some("/tmp/attacker-helper".into()),
            4 => plan.bootstrap_helper_install_path = None,
            _ => unreachable!(),
        }
        plan.plan_blake3 = plan.computed_digest();
        let changed = serde_json::to_vec(&plan).unwrap();
        assert!(
            verify_root_b0_v3_scope(&changed, &claim, &signature, pins, NOW).is_err(),
            "rehashed variant {variant}",
        );
    }
}

#[test]
fn independent_root_pins_cannot_be_replaced_by_signed_candidate() {
    let (raw, claim, signature, trusted) = fixture();
    for variant in 0..9 {
        let mut pins = trusted;
        match variant {
            0 => pins.ssh_host_sha256[0] ^= 1,
            1 => pins.bootstrap_helper_sha256[0] ^= 1,
            2 => pins.node_sha256[0] ^= 1,
            3 => pins.manifest_fingerprint[0] ^= 1,
            4 => pins.owner_ed25519[0] ^= 1,
            5 => pins.release_signer_id = "fake-publisher",
            6 => pins.release_version = "9.9.9",
            7 => pins.host_arch = MachineArch::Aarch64,
            8 => pins.node_size = 2048,
            _ => unreachable!(),
        }
        assert!(
            verify_root_b0_v3_scope(&raw, &claim, &signature, pins, NOW).is_err(),
            "independent pin variant {variant}",
        );
    }
}

#[test]
fn valid_signature_on_another_plan_does_not_override_pinned_scope() {
    let (raw, mut claim, _, pins) = fixture();
    claim.plan_digest[0] ^= 1;
    let resigned = signed_claim(&claim);
    assert_eq!(
        verify_root_b0_v3_scope(&raw, &claim, &resigned, pins, NOW),
        Err(RootPlanError::WrongOwnerScope),
    );
}

#[test]
fn replay_stale_revision_bad_signature_and_expiry_hold() {
    let (raw, claim, signature, pins) = fixture();
    let mut stale = pins;
    stale.witness_revision = 1;
    assert_eq!(
        verify_root_b0_v3_scope(&raw, &claim, &signature, stale, NOW),
        Err(RootPlanError::StaleWitness),
    );
    let mut forgery = signature;
    forgery[0] ^= 1;
    assert_eq!(
        verify_root_b0_v3_scope(&raw, &claim, &forgery, pins, NOW),
        Err(RootPlanError::OwnerSignature(WireError::BadSignature)),
    );
    assert_eq!(
        verify_root_b0_v3_scope(&raw, &claim, &signature, pins, NOW + 40),
        Err(RootPlanError::OwnerSignature(WireError::Expired)),
    );
}
