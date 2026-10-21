//! Reuse the B1b4a fixture for OS-pinned CAS tests; no install actions.
//! No user host and no installation is involved.
#![cfg(all(target_os="linux",feature="server-bootstrap-b1c0-fixed-node-lab"))]
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
    fn new()->Self{
        assert_eq!(rustix::process::geteuid().as_raw(),0);
        let dir=tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(),Permissions::from_mode(0o700)).unwrap();
        let rk=key(0x31);
        let hash:[u8;32]=digest::digest(&digest::SHA256,NODE)
            .as_ref().try_into().unwrap();
        let m=ReleaseManifestV1{
            platform:TargetPlatform::LinuxX86_64,
            version:Version{major:1,minor:0,patch:0},
            release_sequence:17,package_size:NODE.len() as u64,
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
        write(&dir.path().join("node"),NODE);
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
        let plan=ApprovedPlanDigest([0x22;32]);
        let stage=StageReceipt{
            install_id,approved_plan_digest:plan,
            verified_package:pinned.receipt.clone(),idempotent_replay:false,
        };
        let binding=LabPlanBinding{
            install_id,plan_digest:plan,pinned_host_key_sha256:[0x33;32],
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
            staged_package_size:NODE.len() as u64,expected_witness_revision:0,
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

use aquilum_core::server_installer_fixed_node_lab::{
    commit_first_signed_node_in_ci, FixedNodeError,
};
use std::os::unix::fs::MetadataExt;
const ROOT_CI: &str = "/run/akula-b1c0-one-shot-ci";
const MARKER_CI: &[u8] = b"AKULA-B1C0-ENROLLED-CI-V1\n";
struct IsolatedRootTarget;
impl IsolatedRootTarget {
    fn new() -> Self {
        assert_eq!(std::env::var("GITHUB_ACTIONS").ok().as_deref(), Some("true"));
        assert_eq!(std::env::var("RUNNER_OS").ok().as_deref(), Some("Linux"));
        assert_eq!(rustix::process::geteuid().as_raw(), 0);
        let root = std::path::Path::new(ROOT_CI);
        assert!(!root.exists(), "refuse to touch a pre-existing OS path");
        fs::create_dir(root).unwrap();
        fs::set_permissions(root, Permissions::from_mode(0o711)).unwrap();
        let marker = root.join("enrollment");
        fs::write(&marker, MARKER_CI).unwrap();
        fs::set_permissions(&marker, Permissions::from_mode(0o600)).unwrap();
        let target = root.join("target");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, Permissions::from_mode(0o755)).unwrap();
        Self
    }
    fn path(&self) -> std::path::PathBuf {
        std::path::Path::new(ROOT_CI).join("target")
    }
}
impl Drop for IsolatedRootTarget {
    fn drop(&mut self) {
        // The caller created this known empty disposable namespace and is
        // running as root on a CI runner. Never clean user-chosen paths.
        fs::remove_dir_all(ROOT_CI).expect("disposable root cleanup");
    }
}
fn commit(
    c: &Case, actor: &OsWitnessLab,
) -> Result<aquilum_core::server_installer_fixed_node_lab::FixedNodeReceipt, FixedNodeError> {
    commit_first_signed_node_in_ci(
        &c.pinned, actor, &c.owner, &c.owner_signer,
        c.scope, &c.stage, c.proposed, c.before, c.challenge, NOW,
    )
}
#[test]
fn actual_owner_cas_and_root_pinned_bytes_commit_once_to_fixed_name() {
    let sandbox = IsolatedRootTarget::new();
    let c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    let result = commit(&c, &actor).unwrap();
    assert!(result.no_activation_was_performed);
    assert_eq!(result.witness_revision, 1);
    assert_eq!(result.installed_byte_length, NODE.len() as u64);
    assert_eq!(result.installed_sha256, c.pinned.receipt.package_sha256);
    let file = sandbox.path().join("node");
    assert_eq!(fs::read(&file).unwrap(), NODE);
    let metadata = fs::metadata(&file).unwrap();
    assert_eq!(metadata.uid(), 0);
    assert_eq!(metadata.permissions().mode() & 0o7777, 0o555);
    assert_eq!(metadata.nlink(), 1);
    assert_eq!(fs::read_dir(sandbox.path()).unwrap().count(), 1);
    assert_eq!(actor.current_checkpoint().unwrap().revision, 1);
}
#[test]
fn replay_preserves_exact_installed_bytes_and_old_revision() {
    let sandbox = IsolatedRootTarget::new();
    let c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    commit(&c, &actor).unwrap();
    assert!(matches!(commit(&c, &actor), Err(FixedNodeError::ExistingOrInterrupted)));
    assert_eq!(fs::read(sandbox.path().join("node")).unwrap(), NODE);
    assert_eq!(actor.current_checkpoint().unwrap().revision, 1);
}
#[test]
fn preexisting_node_refuses_before_any_cas_and_never_overwrites() {
    let sandbox = IsolatedRootTarget::new();
    fs::write(sandbox.path().join("node"), b"prior working release").unwrap();
    let c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    assert!(matches!(commit(&c, &actor), Err(FixedNodeError::ExistingOrInterrupted)));
    assert_eq!(fs::read(sandbox.path().join("node")).unwrap(), b"prior working release");
    assert_eq!(actor.current_checkpoint().unwrap().revision, 0);
}
#[test]
fn partial_pending_file_causes_repair_hold_before_consent_is_consumed() {
    let sandbox = IsolatedRootTarget::new();
    fs::write(sandbox.path().join("node.pending"), b"interrupted payload").unwrap();
    let c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    assert!(matches!(commit(&c, &actor), Err(FixedNodeError::ExistingOrInterrupted)));
    assert_eq!(actor.current_checkpoint().unwrap(), WitnessCheckpoint::genesis());
    assert!(!sandbox.path().join("node").exists());
}
#[test]
fn symlink_destination_is_refused_without_touching_victim() {
    let sandbox = IsolatedRootTarget::new();
    std::os::unix::fs::symlink("/etc/passwd", sandbox.path().join("node")).unwrap();
    let before = fs::read("/etc/passwd").unwrap();
    let c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    assert!(matches!(commit(&c, &actor), Err(FixedNodeError::ExistingOrInterrupted)));
    assert_eq!(before, fs::read("/etc/passwd").unwrap());
    assert_eq!(actor.current_checkpoint().unwrap().revision, 0);
}
#[test]
fn invalid_detached_owner_signature_does_not_commit_cas_or_file() {
    let sandbox = IsolatedRootTarget::new();
    let mut c = Case::new();
    c.owner.signature[0] ^= 1;
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    assert!(matches!(commit(&c, &actor),
        Err(FixedNodeError::Witness(OsWitnessError::Witness(
            WitnessError::Approval(ApprovalError::BadSignature))))));
    assert_eq!(actor.current_checkpoint().unwrap().revision, 0);
    assert!(fs::read_dir(sandbox.path()).unwrap().next().is_none());
}
#[test]
fn pinned_file_changed_after_signature_validation_is_refused_before_cas() {
    let sandbox = IsolatedRootTarget::new();
    let c = Case::new();
    let node = c._dir.path().join("node");
    fs::set_permissions(&node, Permissions::from_mode(0o600)).unwrap();
    fs::write(&node, b"forged bytes with a different size").unwrap();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    assert!(matches!(commit(&c, &actor), Err(FixedNodeError::Pin(PinError::ChangedInode))));
    assert_eq!(actor.current_checkpoint().unwrap().revision, 0);
    assert!(!sandbox.path().join("node").exists());
}
#[test]
fn wrong_plan_scope_or_bad_challenge_refuses_without_creating_node() {
    let sandbox = IsolatedRootTarget::new();
    let mut c = Case::new();
    c.scope.pinned_host_key_sha256[0] ^= 1;
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    assert!(commit(&c, &actor).is_err());
    assert_eq!(actor.current_checkpoint().unwrap().revision, 0);
    c.scope.pinned_host_key_sha256[0] ^= 1;
    c.challenge = [0;32];
    assert!(matches!(commit(&c, &actor),
        Err(FixedNodeError::Witness(OsWitnessError::InvalidChallenge))));
    assert_eq!(actor.current_checkpoint().unwrap().revision, 0);
    assert!(fs::read_dir(sandbox.path()).unwrap().next().is_none());
}
#[test]
fn missing_enrollment_marker_refuses_even_a_valid_signed_owner() {
    let sandbox = IsolatedRootTarget::new();
    fs::remove_file(std::path::Path::new(ROOT_CI).join("enrollment")).unwrap();
    let c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    assert!(commit(&c, &actor).is_err());
    assert_eq!(actor.current_checkpoint().unwrap().revision, 0);
    assert!(fs::read_dir(sandbox.path()).unwrap().next().is_none());
}
#[test]
fn expired_signed_source_refuses_before_any_write_or_cas() {
    let sandbox = IsolatedRootTarget::new();
    let c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    assert!(matches!(commit_first_signed_node_in_ci(
        &c.pinned, &actor, &c.owner, &c.owner_signer,
        c.scope, &c.stage, c.proposed, c.before, c.challenge, NOW+120,
    ), Err(FixedNodeError::Pin(PinError::Release(ReleaseError::Expired)))));
    assert_eq!(actor.current_checkpoint().unwrap().revision, 0);
    assert!(fs::read_dir(sandbox.path()).unwrap().next().is_none());
}


// B1c1: disposable Ubuntu QEMU integration, deliberately opt-in. The public
// B1a key is pre-provisioned by guest cloud-init, NOT accepted from staged
// release/SSH. This proves exact original signing key + REAL ELF + CAS file
// commit. Owner consent and witness keys remain NONPRODUCTION test signers.
const B1C1_DOMAIN: &[u8] = b"Akula Node B1a signed release manifest v1\0";
fn b1c1_take<const N: usize>(input: &[u8], at: &mut usize) -> [u8; N] {
    let end = at.checked_add(N).expect("wire offset overflow");
    let part: [u8; N] = input.get(*at..end)
        .expect("truncated external signed manifest")
        .try_into().expect("exact B1a field width");
    *at = end;
    part
}
fn b1c1_u32(input: &[u8], at: &mut usize) -> u32 {
    u32::from_be_bytes(b1c1_take::<4>(input, at))
}
fn b1c1_u64(input: &[u8], at: &mut usize) -> u64 {
    u64::from_be_bytes(b1c1_take::<8>(input, at))
}
fn b1c1_manifest(wire: &[u8]) -> ReleaseManifestV1 {
    assert!(wire.len() <= 256 && wire.starts_with(B1C1_DOMAIN),
        "external B1a signing domain must match");
    let mut at = B1C1_DOMAIN.len();
    assert_eq!(b1c1_take::<1>(wire, &mut at), [1], "schema");
    assert_eq!(b1c1_take::<1>(wire, &mut at), [1], "Linux platform");
    let m = ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version: Version {
            major: b1c1_u32(wire, &mut at),
            minor: b1c1_u32(wire, &mut at),
            patch: b1c1_u32(wire, &mut at),
        },
        release_sequence: b1c1_u64(wire, &mut at),
        package_size: b1c1_u64(wire, &mut at),
        not_before_unix: b1c1_u64(wire, &mut at),
        expires_unix: b1c1_u64(wire, &mut at),
        package_sha256: b1c1_take::<32>(wire, &mut at),
        signer_id: {
            let len = b1c1_take::<1>(wire, &mut at)[0] as usize;
            let end = at.checked_add(len).expect("signer size overflow");
            let id = std::str::from_utf8(wire.get(at..end)
                .expect("truncated B1a signer")).expect("signer UTF-8").to_owned();
            at = end;
            id
        },
    };
    assert_eq!(at, wire.len(), "unexpected unsigned manifest suffix");
    assert_eq!(m.canonical_signed_bytes(), wire, "noncanonical B1a wire");
    assert_eq!(m.signer_id, "akula-b1b2b-ephemeral-test");
    m
}
#[test]
fn ci_real_preprovisioned_signed_elf_b1c1() {
    if std::env::var("AKULA_B1C1_VM_PROOF").ok().as_deref()
        != Some("SIGNED_ELF_TO_FIXED_NODE_CAS_ONLY") { return; }
    assert_eq!(std::env::var("GITHUB_ACTIONS").ok().as_deref(), Some("true"));
    assert_eq!(std::env::var("RUNNER_OS").ok().as_deref(), Some("Linux"));
    assert_eq!(rustix::process::geteuid().as_raw(), 0);
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let trusted_pub = std::path::Path::new("/etc/akula-b1b2b-lab-public.ed25519");
    let marker = std::path::Path::new("/etc/akula-b1b2b-lab.marker");
    for path in [trusted_pub, marker] {
        let meta = fs::symlink_metadata(path).expect("missing provisioned root trust");
        assert!(meta.is_file() && meta.uid() == 0 && meta.nlink() == 1
            && meta.permissions().mode() & 0o7777 == 0o600,
            "root-controlled immutable guest provisioner only");
    }
    let source = std::path::Path::new("/home/akulavm/b1b2b");
    let actual_elf = fs::read(source.join("v1.node")).expect("real signed Node ELF");
    assert!(actual_elf.len() >= 1024 && actual_elf.starts_with(b"\x7fELF"),
        "real Linux ELF required, synthetic bytes are forbidden");
    let wire = fs::read(source.join("v1.manifest")).unwrap();
    let manifest = b1c1_manifest(&wire);
    let signature: [u8; 64] = fs::read(source.join("v1.sig")).unwrap()
        .try_into().expect("exact Ed25519 signature");
    let trusted_public_key: [u8; 32] = fs::read(trusted_pub).unwrap()
        .try_into().expect("pre-boot provisioned public key");
    let signed = SignedRelease { manifest, signature };
    let trusted = TrustedReleaseSigner {
        signer_id: signed.manifest.signer_id.clone(), ed25519_public: trusted_public_key,
    };
    let verified = verify_signed_release(
        &signed, &trusted, ReleaseFloor::initial(), now,
    ).expect("external root-pinned B1a signature and freshness");
    assert_eq!(verified.manifest().package_size, actual_elf.len() as u64);
    let original_digest = digest::digest(&digest::SHA256, &actual_elf);
    assert_eq!(verified.manifest().package_sha256.as_slice(),
        original_digest.as_ref(), "signed bytes must be this ELF");

    // Reuse the existing independently tested signed-owner/CAS fixture.
    // Replace its TEST synthetic B1a package ONLY after authenticating the
    // actual VM release using pre-boot trusted public key above.
    let sandbox = IsolatedRootTarget::new();
    let mut c = Case::new();
    let stage_root = c._dir.path();
    write(&stage_root.join("node"), &actual_elf);
    write(&stage_root.join("manifest"), &wire);
    write(&stage_root.join("signature"), &signature);
    let pinned_trust = RootPinnedReleaseTrust {
        trusted_signer: trusted,
        previous_floor: ReleaseFloor::initial(),
        approved_manifest_fingerprint:
            verified.release_floor_candidate().signed_manifest_fingerprint,
        approved_package_sha256: verified.manifest().package_sha256,
        approved_release_sequence: verified.manifest().release_sequence,
    };
    c.pinned = verify_root_staged(stage_root, &pinned_trust, now)
        .expect("pin the exact B1a signed root-staged VM inode");
    c.stage.verified_package = c.pinned.receipt.clone();
    c.proposed.proposed_head.floor = c.pinned.receipt.proposed_floor;
    c.owner.claim.scope_binding.signed_manifest_fingerprint =
        c.pinned.receipt.signed_manifest_fingerprint;
    c.owner.claim.proposed_head = c.proposed.proposed_head;
    c.owner.claim.staged_package_sha256 = c.pinned.receipt.package_sha256;
    c.owner.claim.staged_package_size = c.pinned.receipt.byte_length;
    c.owner.claim.issued_at_unix = now.saturating_sub(5);
    c.owner.claim.expires_at_unix = now.checked_add(30).unwrap();
    c.owner.signature = key(0x32).sign(&c.owner.claim.canonical_bytes())
        .as_ref().try_into().unwrap();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    let receipt = commit_first_signed_node_in_ci(
        &c.pinned, &actor, &c.owner, &c.owner_signer, c.scope,
        &c.stage, c.proposed, c.before, c.challenge, now,
    ).expect("real signed ELF + detached synthetic owner + OS-owned CAS commit");
    assert!(receipt.no_activation_was_performed);
    assert_eq!(receipt.witness_revision, 1);
    assert_eq!(receipt.installed_sha256, verified.manifest().package_sha256);
    let installed = sandbox.path().join("node");
    let installed_metadata = fs::metadata(&installed).unwrap();
    assert_eq!(installed_metadata.uid(), 0);
    assert_eq!(installed_metadata.permissions().mode() & 0o7777, 0o555);
    assert_eq!(installed_metadata.len(), actual_elf.len() as u64);
    assert_eq!(fs::read(&installed).unwrap(), actual_elf,
        "byte-exact real ELF from signed B1a release");
    assert_eq!(actor.current_checkpoint().unwrap().revision, 1);
    println!("B1C1_GUEST_ROOT_PINNED_ACTUAL_SIGNED_ELF_CAS_COMMIT_PASS");
    // Guest-only: keep the fixed executable until the host's separate
    // systemd activation/health probe completes and explicitly cleans it.
    std::mem::forget(sandbox);
}
