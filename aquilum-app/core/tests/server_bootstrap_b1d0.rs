//! Reuse the B1b4a fixture for OS-pinned CAS tests; no install actions.
//! No user host and no installation is involved.
#![cfg(all(target_os="linux",feature="server-bootstrap-b1d0-durable-node"))]
use aquilum_core::server_installer_cas_fence_lab::{
    check_inert_cas_fence,signed_owner_sha256,FenceError,
    InertFenceEvidence,SignedWitnessClaim,TrustedWitnessSigner,WitnessClaim,
};
use aquilum_core::server_installer_pinned_release_lab::{
    verify_root_staged,PinError,RootPinnedReleaseTrust,RootVerifiedFd,
};
use aquilum_core::server_owner_approval_lab::{
    ApprovalError,ExternalApprovalScope,SignedApprovalClaim,
    SignedOwnerApproval,TrustedOwnerApprover,
};
use aquilum_core::server_release_floor_lab::{
    LabPlanBinding,ProposedFloor,TrustedFloorHead,
};
use aquilum_core::server_release_lab::{
    verify_signed_release,ReleaseError,ReleaseFloor,ReleaseManifestV1,
    SignedRelease,TargetPlatform,TrustedReleaseSigner,Version,
};
use aquilum_core::server_staging_lab::{ApprovedPlanDigest,InstallId,StageReceipt};
use aquilum_core::server_witness_lab::WitnessCheckpoint;
use ring::{digest,signature::{self,KeyPair}};
use std::fs::{self,Permissions};
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;
const NOW:u64=1_800_000_000;
const CHALLENGE:[u8;32]=[0x90;32];
const NODE:&[u8]=b"synthetic signed root pinned Akula ELF-like bytes never executed";
fn key(v:u8)->signature::Ed25519KeyPair {
    signature::Ed25519KeyPair::from_seed_unchecked(&[v;32]).unwrap()
}
fn write(p:&std::path::Path,b:&[u8]){
    fs::write(p,b).unwrap();
    fs::set_permissions(p,Permissions::from_mode(0o400)).unwrap();
}
struct Case {
    _dir:TempDir,pinned:RootVerifiedFd,
    owner:SignedOwnerApproval,owner_signer:TrustedOwnerApprover,
    scope:ExternalApprovalScope,stage:StageReceipt,proposed:ProposedFloor,
    witness:SignedWitnessClaim,witness_signer:TrustedWitnessSigner,
    before:WitnessCheckpoint,after:WitnessCheckpoint,challenge:[u8;32],
}
impl Case {
    fn new()->Self{ Self::with_package(NODE) }
    fn with_package(package: &[u8])->Self {
        Self::with_package_bound(
            package, ApprovedPlanDigest([0x22; 32]), [0x33; 32],
        )
    }
    /// Same existing B1d0 signer/witness/CAS fixture, but with the exact
    /// immutable B0 v3 digest and pinned SSH host from a real preview.
    fn with_package_bound(
        package: &[u8],
        plan: ApprovedPlanDigest,
        pinned_host_sha256: [u8; 32],
    ) -> Self {
        assert_eq!(rustix::process::geteuid().as_raw(),0);
        let dir=tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(),Permissions::from_mode(0o700)).unwrap();
        let rk=key(0x31);
        let hash:[u8;32]=digest::digest(&digest::SHA256,package)
            .as_ref().try_into().unwrap();
        let m=ReleaseManifestV1{
            platform:TargetPlatform::LinuxX86_64,
            version:Version{major:1,minor:0,patch:0},
            release_sequence:17,package_size:package.len() as u64,
            not_before_unix:NOW-60,expires_unix:NOW+120,
            package_sha256:hash,signer_id:"b1b4a-test-release".into(),
        };
        let sig:[u8;64]=rk.sign(&m.canonical_signed_bytes())
            .as_ref().try_into().unwrap();
        let signer=TrustedReleaseSigner{
            signer_id:m.signer_id.clone(),
            ed25519_public:rk.public_key().as_ref().try_into().unwrap(),
        };
        let release=verify_signed_release(
            &SignedRelease{manifest:m.clone(),signature:sig},
            &signer,ReleaseFloor::initial(),NOW).unwrap();
        write(&dir.path().join("node"),package);
        write(&dir.path().join("manifest"),&m.canonical_signed_bytes());
        write(&dir.path().join("signature"),&sig);
        let trust=RootPinnedReleaseTrust{
            trusted_signer:signer,previous_floor:ReleaseFloor::initial(),
            approved_manifest_fingerprint:
                release.release_floor_candidate().signed_manifest_fingerprint,
            approved_package_sha256:hash,approved_release_sequence:17,
        };
        let pinned=verify_root_staged(dir.path(),&trust,NOW).unwrap();
        let install_id=InstallId([0x11;16]);
        let stage=StageReceipt{
            install_id,approved_plan_digest:plan,
            verified_package:pinned.receipt.clone(),idempotent_replay:false,
        };
        let binding=LabPlanBinding{
            install_id,plan_digest:plan,pinned_host_key_sha256:pinned_host_sha256,
            signed_manifest_fingerprint:pinned.receipt.signed_manifest_fingerprint,
        };
        let before=WitnessCheckpoint::genesis();
        let newhead=TrustedFloorHead{
            generation:1,floor:pinned.receipt.proposed_floor,chain_tip:[0x44;32],
        };
        let proposed=ProposedFloor{
            proposed_head:newhead,idempotent_replay:false,
            requires_external_witness:true,
        };
        let scope=ExternalApprovalScope{
            install_id,approved_plan_digest:plan,
            pinned_host_key_sha256:binding.pinned_host_key_sha256,
            independently_pinned_previous_head:before.accepted_floor,
            expected_witness_revision:0,
        };
        let ok=key(0x32);
        let owner_signer=TrustedOwnerApprover{
            approver_id:"b1b4a-test-owner".into(),
            public_key_ed25519:ok.public_key().as_ref().try_into().unwrap(),
        };
        let claim=SignedApprovalClaim{
            approver_id:owner_signer.approver_id.clone(),
            scope_binding:binding,previous_head:before.accepted_floor,
            proposed_head:newhead,staged_package_sha256:hash,
            staged_package_size:package.len() as u64,expected_witness_revision:0,
            approval_nonce:[0x55;16],
            issued_at_unix:NOW-10,expires_at_unix:NOW+45,
        };
        let owner=SignedOwnerApproval{
            signature:ok.sign(&claim.canonical_bytes()).as_ref().try_into().unwrap(),
            claim,
        };
        let after=WitnessCheckpoint{
            revision:1,accepted_floor:newhead,record_tip:[0x66;32],
        };
        let wk=key(0x34);
        let witness_signer=TrustedWitnessSigner{
            witness_id:"independent-ci-witness".into(),
            ed25519_public:wk.public_key().as_ref().try_into().unwrap(),
        };
        let wc=WitnessClaim{
            witness_id:witness_signer.witness_id.clone(),
            before,after,install_id:install_id.0,plan_digest:plan.0,
            host_sha256:binding.pinned_host_key_sha256,
            manifest_fingerprint:pinned.receipt.signed_manifest_fingerprint,
            package_sha256:hash,signed_owner_sha256:signed_owner_sha256(&owner),
            approval_nonce:owner.claim.approval_nonce,challenge:CHALLENGE,
            issued_at_unix:NOW-5,expires_at_unix:NOW+20,
        };
        let witness=SignedWitnessClaim{
            signature:wk.sign(&wc.canonical_bytes()).as_ref().try_into().unwrap(),
            claim:wc,
        };
        Self{_dir:dir,pinned,owner,owner_signer,scope,stage,proposed,
            witness,witness_signer,before,after,challenge:CHALLENGE}
    }
    fn check(&self,now:u64)->Result<InertFenceEvidence,FenceError>{
        check_inert_cas_fence(
            &self.pinned,&self.owner,&self.owner_signer,self.scope,
            &self.stage,self.proposed,&self.witness,&self.witness_signer,
            self.before,self.after,self.challenge,now)
    }
    fn resign(&mut self){
        self.witness.signature=key(0x34)
            .sign(&self.witness.claim.canonical_bytes()).as_ref().try_into().unwrap();
    }
}

use aquilum_core::server_installer_os_witness_lab::{OsWitnessError, OsWitnessLab};
use aquilum_core::server_witness_lab::WitnessError;
use std::path::Path;

fn provision() -> (TempDir, TempDir) {
    let trust = tempfile::tempdir().unwrap();
    let journal = tempfile::tempdir().unwrap();
    for dir in [&trust, &journal] {
        fs::set_permissions(dir.path(), Permissions::from_mode(0o700)).unwrap();
    }
    for (name, bytes) in [
        ("witness.seed", vec![0x34; 32]),
        ("witness.pub", key(0x34).public_key().as_ref().to_vec()),
        ("enrollment", b"AKULA-B1B4B-CI-ENROLLED\n".to_vec()),
    ] {
        let path = trust.path().join(name);
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
    }
    (trust, journal)
}
fn open_actor(trust: &Path, journal: &Path) -> OsWitnessLab {
    OsWitnessLab::open(trust, journal).unwrap()
}


use aquilum_core::server_installer_durable_node::{
    commit_first_ci_opt, inspect_ci_hold, inspect_provisioned_opt_hold,
    CrashPoint, DurableError, DurableStatus,
};
use std::os::unix::fs::MetadataExt;

const CI_ROOT: &str = "/opt/akula-b1d0-ci-node";
struct RootFixture;
impl RootFixture {
    fn new() -> Self {
        assert_eq!(rustix::process::geteuid().as_raw(), 0);
        assert_eq!(std::env::var("GITHUB_ACTIONS").ok().as_deref(), Some("true"));
        assert_eq!(std::env::var("RUNNER_OS").ok().as_deref(), Some("Linux"));
        assert_eq!(std::env::var("AKULA_B1D0_CI").ok().as_deref(),
            Some("STRICT_ROOT_ONE_SHOT_FILE_ONLY"));
        let root = Path::new(CI_ROOT);
        assert!(fs::symlink_metadata(root).is_err(), "must not reuse an OS path");
        fs::create_dir(root).unwrap();
        fs::set_permissions(root, Permissions::from_mode(0o755)).unwrap();
        fs::write(root.join("enrollment"),
            b"AKULA-B1D0-ISOLATED-CI-FIRST-INSTALL-V1\n").unwrap();
        fs::set_permissions(root.join("enrollment"),
            Permissions::from_mode(0o600)).unwrap();
        Self
    }
    fn entry(&self, name: &str) -> std::path::PathBuf {
        Path::new(CI_ROOT).join(name)
    }
}
impl Drop for RootFixture {
    fn drop(&mut self) {
        let dir = Path::new(CI_ROOT);
        assert!(dir.is_dir() && !dir.is_symlink());
        // Never recursively delete an untrusted tree: names are exact and
        // this root is created in disposable CI and never reused.
        for name in ["node", "node.pending", "node.receipt",
                     "node.receipt.pending", "enrollment"] {
            let p = dir.join(name);
            if fs::symlink_metadata(&p).is_ok() {
                fs::remove_file(&p).expect("remove exact disposable file");
            }
        }
        fs::remove_dir(dir).expect("no foreign entries allowed in test root");
    }
}
fn commit(c: &Case, actor: &OsWitnessLab, point: CrashPoint)
    -> Result<DurableStatus, DurableError>
{
    commit_first_ci_opt(
        &c.pinned, actor, &c.owner, &c.owner_signer, c.scope,
        &c.stage, c.proposed, c.before, c.challenge, NOW, point,
    )
}

#[test]
fn actual_root_signed_fd_owner_cas_writes_durable_opt_and_reopens_as_hold() {
    let dir = RootFixture::new();
    let c = Case::new();
    let (trust, ledger) = provision();
    let actor = open_actor(trust.path(), ledger.path());
    assert_eq!(inspect_ci_hold(&actor).unwrap(), DurableStatus::Empty);
    let wanted = DurableStatus::LocallyConsistentHold {
        revision: 1, sha256: c.pinned.receipt.package_sha256,
    };
    assert_eq!(commit(&c, &actor, CrashPoint::None).unwrap(), wanted);
    assert_eq!(fs::read(dir.entry("node")).unwrap(), NODE);
    let m = fs::metadata(dir.entry("node")).unwrap();
    assert_eq!((m.uid(), m.nlink(), m.permissions().mode() & 0o7777),
        (0, 1, 0o555));
    assert_eq!(fs::metadata(dir.entry("node.receipt")).unwrap()
        .permissions().mode() & 0o7777, 0o600);
    drop(actor);
    let reopened = open_actor(trust.path(), ledger.path());
    assert_eq!(inspect_ci_hold(&reopened).unwrap(), wanted);
    assert_eq!(reopened.current_checkpoint().unwrap().revision, 1);
}
#[test]
fn replay_cannot_overwrite_node_or_reuse_consumed_owner_nonce() {
    let dir = RootFixture::new();
    let c = Case::new();
    let (trust, ledger) = provision();
    let actor = open_actor(trust.path(), ledger.path());
    commit(&c, &actor, CrashPoint::None).unwrap();
    let old = fs::read(dir.entry("node")).unwrap();
    assert!(matches!(commit(&c, &actor, CrashPoint::None),
        Err(DurableError::NeedsRepair)));
    assert_eq!(fs::read(dir.entry("node")).unwrap(), old);
    assert_eq!(actor.current_checkpoint().unwrap().revision, 1);
}
#[test]
fn every_post_cas_fsync_boundary_is_an_explicit_repair_hold() {
    for point in [
        CrashPoint::AfterCas, CrashPoint::AfterPendingSync,
        CrashPoint::AfterNodeRename, CrashPoint::AfterReceiptPendingSync
    ] {
        let _root = RootFixture::new();
        let c = Case::new();
        let (trust, ledger) = provision();
        let actor = open_actor(trust.path(), ledger.path());
        assert!(matches!(commit(&c, &actor, point),
            Err(DurableError::InjectedStop)), "point: {point:?}");
        assert_eq!(actor.current_checkpoint().unwrap().revision, 1);
        assert!(matches!(inspect_ci_hold(&actor), Err(DurableError::NeedsRepair)));
        let resumed = open_actor(trust.path(), ledger.path());
        assert!(matches!(inspect_ci_hold(&resumed), Err(DurableError::NeedsRepair)));
    }
}
#[test]
fn foreign_existing_node_refuses_before_consuming_cas() {
    let dir = RootFixture::new();
    fs::write(dir.entry("node"), b"protected previous release").unwrap();
    let c = Case::new();
    let (trust, ledger) = provision();
    let actor = open_actor(trust.path(), ledger.path());
    assert!(matches!(commit(&c, &actor, CrashPoint::None),
        Err(DurableError::NeedsRepair)));
    assert_eq!(actor.current_checkpoint().unwrap(), WitnessCheckpoint::genesis());
    assert_eq!(fs::read(dir.entry("node")).unwrap(), b"protected previous release");
}
#[test]
fn bad_root_enrollment_and_forged_owner_refuse_before_writing() {
    {
        let dir = RootFixture::new();
        fs::set_permissions(dir.entry("enrollment"),
            Permissions::from_mode(0o644)).unwrap();
        let c = Case::new();
        let (trust, ledger) = provision();
        let actor = open_actor(trust.path(), ledger.path());
        assert!(matches!(commit(&c, &actor, CrashPoint::None),
            Err(DurableError::Unenrolled)));
        assert_eq!(actor.current_checkpoint().unwrap().revision, 0);
    }
    {
        let dir = RootFixture::new();
        let mut c = Case::new();
        c.owner.signature[0] ^= 1;
        let (trust, ledger) = provision();
        let actor = open_actor(trust.path(), ledger.path());
        assert!(matches!(commit(&c, &actor, CrashPoint::None),
            Err(DurableError::FileCommit(_))));
        assert!(!dir.entry("node").exists());
        assert_eq!(actor.current_checkpoint().unwrap().revision, 0);
    }
}
#[test]
fn tampered_receipt_and_changed_node_are_never_accepted_as_healthy() {
    {
        let dir = RootFixture::new();
        let c = Case::new();
        let (trust, ledger) = provision();
        let actor = open_actor(trust.path(), ledger.path());
        commit(&c, &actor, CrashPoint::None).unwrap();
        let path = dir.entry("node.receipt");
        let mut bytes = fs::read(&path).unwrap();
        bytes[25] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(matches!(inspect_ci_hold(&actor),
            Err(DurableError::InvalidReceipt)));
    }
    {
        let dir = RootFixture::new();
        let c = Case::new();
        let (trust, ledger) = provision();
        let actor = open_actor(trust.path(), ledger.path());
        commit(&c, &actor, CrashPoint::None).unwrap();
        let path = dir.entry("node");
        fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, b"modified after first signed install").unwrap();
        assert!(matches!(inspect_ci_hold(&actor),
            Err(DurableError::NeedsRepair)));
    }
}
#[test]
fn symlink_target_is_denied_without_modifying_the_victim() {
    let dir = RootFixture::new();
    std::os::unix::fs::symlink("/etc/passwd",dir.entry("node")).unwrap();
    let before = fs::read("/etc/passwd").unwrap();
    let c = Case::new();
    let (trust, ledger) = provision();
    let actor = open_actor(trust.path(), ledger.path());
    assert!(matches!(commit(&c, &actor, CrashPoint::None),
        Err(DurableError::NeedsRepair)));
    assert_eq!(before, fs::read("/etc/passwd").unwrap());
    assert_eq!(actor.current_checkpoint().unwrap().revision, 0);
}

#[test]
fn b1d0_actual_built_linux_node_elf_uses_one_root_signed_fd_and_opt_receipt() {
    // Executable is compiled from this exact PR before root tests run.
    // Ephemeral CI-only signer signs its actual bytes; no production key.
    let path = std::env::var("AKULA_B1D0_REAL_ELF")
        .expect("real Akula Node path must be provided explicitly by CI");
    let binary = fs::read(&path).expect("real built Node");
    assert!(binary.len() > 1024 && binary.starts_with(b"\x7fELF"),
        "never accept a synthetic fixture in this acceptance");
    assert!(binary.len() < 512 * 1024 * 1024);
    let _root = RootFixture::new();
    let c = Case::with_package(&binary);
    let (trust, ledger) = provision();
    let actor = open_actor(trust.path(), ledger.path());
    let status = commit(&c, &actor, CrashPoint::None)
        .expect("real signed ELF direct /opt owner CAS");
    assert_eq!(status, DurableStatus::LocallyConsistentHold {
        revision: 1, sha256: c.pinned.receipt.package_sha256,
    });
    let installed = fs::read(Path::new(CI_ROOT).join("node")).unwrap();
    assert_eq!(installed, binary,
        "direct /opt install MUST contain the exact signed executable bytes");
    assert_eq!(digest::digest(&digest::SHA256, &installed).as_ref(),
        c.pinned.receipt.package_sha256.as_slice());
    assert!(matches!(commit(&c, &actor, CrashPoint::None),
        Err(DurableError::NeedsRepair)));
    drop(actor);
    let reopened = open_actor(trust.path(), ledger.path());
    assert_eq!(inspect_ci_hold(&reopened).unwrap(), status);
    println!("B1D0_SIGNED_REAL_AKULA_NODE_ELF_DIRECT_OPT_HOLD_PASS");
}

#[test]
fn production_inspection_refuses_unenrolled_os_witness_and_does_not_provision() {
    // This is a disposable GitHub runner, never a user's VPS. A CI witness
    // fixture from /tmp must NOT supply a production /opt install authority.
    assert_eq!(std::env::var("GITHUB_ACTIONS").ok().as_deref(), Some("true"));
    assert_eq!(rustix::process::geteuid().as_raw(), 0);
    let witness_root = Path::new("/var/lib/akula/server-bootstrap");
    let destination = Path::new("/opt/akula-node");
    assert!(fs::symlink_metadata(witness_root).is_err());
    assert!(fs::symlink_metadata(destination).is_err());
    let result = inspect_provisioned_opt_hold();
    assert!(result.is_err(),
        "unprovisioned fixed OS-owned witness cannot authorize a report");
    assert!(fs::symlink_metadata(witness_root).is_err());
    assert!(fs::symlink_metadata(destination).is_err());
}


#[cfg(feature = "server-root-b0v3-gate-lab")]
#[test]
fn b1d0_actual_node_v1_elf_v3_review_owner_scope_cas_reopen_hold() {
    use aquilum_core::server_bootstrap_lab::{
        preview_node_v3, B0Error, B0Input, MachineArch, PackagePin,
        PreviewTransport, ReadOnlyHostFacts, ServerTarget, SupportedOs,
    };
    use aquilum_core::server_owner_approval_wire::{
        OwnerApprovalWireV1, OwnerFloorWire,
    };
    use aquilum_core::server_root_v3_guard::{
        verify_root_b0_v3_scope, RootIndependentPins, RootPlanError,
    };
    struct PinnedKex(Vec<u8>);
    impl PreviewTransport for PinnedKex {
        fn handshake_public_key(
            &mut self, _: &ServerTarget,
        ) -> Result<Vec<u8>, B0Error> {
            Ok(self.0.clone())
        }
        fn inspect_host(&mut self) -> Result<ReadOnlyHostFacts, B0Error> {
            Ok(ReadOnlyHostFacts {
                os: SupportedOs::Ubuntu,
                arch: MachineArch::X86_64,
                has_systemd: true,
                has_sudo_rights: true,
                cpu_count: 2,
                ram_mib: 4096,
                free_disk_mib: 8192,
            })
        }
    }

    // The exact executable is compiled from the current HEAD. Its detached
    // release and owner signatures remain isolated CI-only authorities.
    let path = std::env::var("AKULA_B1D0_REAL_ELF")
        .expect("real Node-V1 ELF path from the isolated CI driver");
    let binary = fs::read(&path).expect("same-HEAD Node-V1 service ELF");
    assert!(binary.len() > 1024 && binary.starts_with(b"\x7fELF"));
    let node_sha: [u8; 32] = digest::digest(&digest::SHA256, &binary)
        .as_ref().try_into().unwrap();
    let wire = b"akula-ci-actual-pinned-ssh-kex-v3".to_vec();
    let host_sha: [u8; 32] = digest::digest(&digest::SHA256, &wire)
        .as_ref().try_into().unwrap();
    let input = B0Input {
        install_id: uuid::Uuid::from_bytes([0x11; 16]),
        target: ServerTarget {
            host: "vps.example.org".into(),
            ssh_port: 22,
            ssh_user: "ubuntu".into(),
            trusted_key_sha256: host_sha,
        },
        package: PackagePin {
            version: "1.0.0".into(),
            package_sha256: node_sha,
            trusted_signer_id: "b1b4a-test-release".into(),
        },
    };
    // Different helper and Node hashes are disclosed under B0 v3; no
    // user-owned VPS or production key is ever used in this acceptance.
    let helper_sha: [u8; 32] = digest::digest(
        &digest::SHA256, b"isolated-ci-only-bootstrap-helper-role",
    ).as_ref().try_into().unwrap();
    let v3 = preview_node_v3(
        &input, helper_sha, &mut PinnedKex(wire), || false,
    ).expect("valid immutable B0 v3 preview");
    assert_eq!(v3.schema_version, 3);
    assert_eq!(v3.bootstrap_helper_sha256, Some(helper_sha));
    assert!(v3.verify_unchanged());
    assert_eq!(v3.package.package_sha256, node_sha);
    let plan_digest = ApprovedPlanDigest(
        *blake3::Hash::from_hex(&v3.plan_blake3).unwrap().as_bytes(),
    );

    let submitted_json = serde_json::to_vec(&v3).unwrap();
    // The root review is now based on the *actual submitted B0 v3 JSON*,
    // not on an arbitrary 32-byte digest provided by an SSH peer.
    let wire_for = |c: &Case| {
        let floor = |head: TrustedFloorHead| OwnerFloorWire {
            generation: head.generation,
            sequence: head.floor.sequence,
            version: [
                head.floor.version.major,
                head.floor.version.minor,
                head.floor.version.patch,
            ],
            manifest_fingerprint: head.floor.signed_manifest_fingerprint,
            chain_tip: head.chain_tip,
        };
        let claim = &c.owner.claim;
        OwnerApprovalWireV1 {
            approver_id: claim.approver_id.clone(),
            install_id: claim.scope_binding.install_id.0,
            plan_digest: claim.scope_binding.plan_digest.0,
            ssh_host_sha256: claim.scope_binding.pinned_host_key_sha256,
            manifest_fingerprint: claim.scope_binding.signed_manifest_fingerprint,
            previous: floor(claim.previous_head),
            proposed: floor(claim.proposed_head),
            staged_package_sha256: claim.staged_package_sha256,
            staged_package_size: claim.staged_package_size,
            expected_witness_revision: claim.expected_witness_revision,
            approval_nonce: claim.approval_nonce,
            issued_at_unix: claim.issued_at_unix,
            expires_at_unix: claim.expires_at_unix,
        }
    };
    let check_root_plan = |c: &Case, revision: u64| {
        let pins = RootIndependentPins {
            owner_id: &c.owner_signer.approver_id,
            owner_ed25519: c.owner_signer.public_key_ed25519,
            ssh_host_sha256: host_sha,
            bootstrap_helper_sha256: helper_sha,
            node_sha256: node_sha,
            node_size: binary.len() as u64,
            manifest_fingerprint: c.pinned.receipt.signed_manifest_fingerprint,
            release_signer_id: "b1b4a-test-release",
            release_version: "1.0.0",
            host_os: SupportedOs::Ubuntu,
            host_arch: MachineArch::X86_64,
            has_systemd: true,
            witness_revision: revision,
        };
        verify_root_b0_v3_scope(
            &submitted_json, &wire_for(c), &c.owner.signature, pins, NOW,
        )
    };

    // An owner can sign another digest, but the independently pinned scope
    // must refuse it BEFORE consuming the durable root-owned CAS revision.
    let _root = RootFixture::new();
    let mut forged = Case::with_package_bound(&binary, plan_digest, host_sha);
    forged.owner.claim.scope_binding.plan_digest = ApprovedPlanDigest([0xee; 32]);
    forged.owner.signature = key(0x32)
        .sign(&forged.owner.claim.canonical_bytes())
        .as_ref().try_into().unwrap();
    let (trust, ledger) = provision();
    let actor = open_actor(trust.path(), ledger.path());
    assert_eq!(
        check_root_plan(&forged, actor.current_checkpoint().unwrap().revision),
        Err(RootPlanError::WrongOwnerScope),
    );
    assert!(matches!(commit(&forged, &actor, CrashPoint::None),
        Err(DurableError::FileCommit(_))));
    assert_eq!(actor.current_checkpoint().unwrap().revision, 0);
    assert!(!Path::new(CI_ROOT).join("node").exists());
    drop(actor);
    drop(forged);

    // A fresh, exactly bound B0 v3 digest and signed owner/receipt may
    // commit the real ELF once in the disposable root-only /opt namespace.
    let exact = Case::with_package_bound(&binary, plan_digest, host_sha);
    let actor = open_actor(trust.path(), ledger.path());
    let checked = check_root_plan(&exact, actor.current_checkpoint().unwrap().revision)
        .expect("root independently revalidates canonical v3 and owner signature");
    assert_eq!(checked.plan_digest, plan_digest.0);
    assert_eq!(checked.bootstrap_helper_sha256, helper_sha);
    assert_eq!(checked.node_sha256, node_sha);
    let hold = commit(&exact, &actor, CrashPoint::None)
        .expect("B0 v3 owner-plan bound B1d0 root file CAS");
    assert_eq!(hold, DurableStatus::LocallyConsistentHold {
        revision: 1,
        sha256: node_sha,
    });
    assert_eq!(fs::read(Path::new(CI_ROOT).join("node")).unwrap(), binary);
    assert!(matches!(commit(&exact, &actor, CrashPoint::None),
        Err(DurableError::NeedsRepair)));
    drop(actor);
    let reopened = open_actor(trust.path(), ledger.path());
    assert_eq!(inspect_ci_hold(&reopened).unwrap(), hold);
    println!("B1D0_B0_V3_SIGNED_REAL_NODE_OWNER_CAS_REOPEN_HOLD_PASS");
}
