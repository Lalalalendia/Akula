//! B1b3c: real Linux UID0 file-descriptor and signed-B1a tests.
//! Runs with sudo on an EPHEMERAL CI runner; no real package activation.
#![cfg(all(target_os="linux",feature="server-bootstrap-b1b3c-pinned-fd-lab"))]
use aquilum_core::server_installer_pinned_release_lab::{
    verify_root_staged,decode_b1a_wire,PinError,RootPinnedReleaseTrust,
};
use aquilum_core::server_release_lab::{
    verify_signed_release,ReleaseFloor,ReleaseError,ReleaseManifestV1,
    SignedRelease,TargetPlatform,TrustedReleaseSigner,Version,
};
use ring::{digest,signature::{Ed25519KeyPair,KeyPair}};
use std::fs::{self,Permissions};
use std::os::unix::fs::{symlink,PermissionsExt};
use std::path::Path;
use tempfile::TempDir;
const NOW:u64=1_800_000_000;

fn write_mode(path:&Path,bytes:&[u8],mode:u32){
    fs::write(path,bytes).unwrap();
    fs::set_permissions(path,Permissions::from_mode(mode)).unwrap();
}
fn setup()->(TempDir,RootPinnedReleaseTrust){
    assert_eq!(rustix::process::geteuid().as_raw(),0,
        "B1b3c requires actual UID0 on ephemeral runner");
    let dir=tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(),Permissions::from_mode(0o700)).unwrap();
    let payload=b"synthetic signed ELF-like bytes - NEVER EXECUTED";
    let hash:[u8;32]=digest::digest(&digest::SHA256,payload).as_ref().try_into().unwrap();
    let pair=Ed25519KeyPair::from_seed_unchecked(&[0xA1;32]).unwrap();
    let manifest=ReleaseManifestV1 {
        platform:TargetPlatform::LinuxX86_64,
        version:Version{major:1,minor:0,patch:0},
        release_sequence:37,package_size:payload.len() as u64,
        not_before_unix:NOW-60,expires_unix:NOW+500,
        package_sha256:hash,signer_id:"b1b3c-synthetic-test".into(),
    };
    let signature:[u8;64]=pair.sign(&manifest.canonical_signed_bytes()).as_ref().try_into().unwrap();
    let signed=SignedRelease{manifest:manifest.clone(),signature};
    let signer=TrustedReleaseSigner{
        signer_id:manifest.signer_id.clone(),
        ed25519_public:pair.public_key().as_ref().try_into().unwrap(),
    };
    let proof=verify_signed_release(&signed,&signer,ReleaseFloor::initial(),NOW).unwrap();
    write_mode(&dir.path().join("node"),payload,0o400);
    write_mode(&dir.path().join("manifest"),&manifest.canonical_signed_bytes(),0o400);
    write_mode(&dir.path().join("signature"),&signature,0o400);
    let anchor=RootPinnedReleaseTrust{
        trusted_signer:signer,previous_floor:ReleaseFloor::initial(),
        approved_manifest_fingerprint:proof.release_floor_candidate().signed_manifest_fingerprint,
        approved_package_sha256:hash,approved_release_sequence:37,
    };
    (dir,anchor)
}
#[test]
fn real_root_signed_release_is_pinned_to_exact_inode(){
    let (dir,trust)=setup();
    let verified=verify_root_staged(dir.path(),&trust,NOW).unwrap();
    assert_eq!(verified.receipt.byte_length,48);
    let old=verified.inode_identity();
    fs::rename(dir.path().join("node"),dir.path().join("retired")).unwrap();
    write_mode(&dir.path().join("node"),b"malicious replacement",0o400);
    // The held FD still refers to the original inode, not the new path.
    assert_eq!(verified.inode_identity(),old);
    // Linux rename changes the original inode's ctime. A conservative
    // metadata guard MUST reject it, even though the FD still pins identity.
    assert!(matches!(verified.stable(),Err(PinError::ChangedInode)));
    assert!(verify_root_staged(dir.path(),&trust,NOW).is_err());
}
#[test]
fn package_byte_tampering_refused(){
    let (dir,trust)=setup();
    fs::set_permissions(dir.path().join("node"),Permissions::from_mode(0o600)).unwrap();
    fs::write(dir.path().join("node"),b"modified ELF-looking bytes").unwrap();
    fs::set_permissions(dir.path().join("node"),Permissions::from_mode(0o400)).unwrap();
    assert!(matches!(verify_root_staged(dir.path(),&trust,NOW),
        Err(PinError::Release(ReleaseError::SizeMismatch))|
        Err(PinError::Release(ReleaseError::PackageDigestMismatch))));
}
#[test]
fn wrong_pinned_release_signer_is_rejected(){
    let (dir,mut trust)=setup();
    trust.trusted_signer.ed25519_public[0]^=1;
    assert!(matches!(verify_root_staged(dir.path(),&trust,NOW),
        Err(PinError::Release(ReleaseError::BadSignature))));
}
#[test]
fn valid_signature_but_different_root_policy_is_denied(){
    let (dir,mut trust)=setup();
    trust.approved_package_sha256[0]^=1;
    assert!(matches!(verify_root_staged(dir.path(),&trust,NOW),
        Err(PinError::WrongPolicyScope)));
}
#[test]
fn earlier_signed_release_cannot_rollback_root_floor(){
    let (dir,mut trust)=setup();
    trust.previous_floor.sequence=38;
    trust.previous_floor.version=Version{major:2,minor:0,patch:0};
    assert!(matches!(verify_root_staged(dir.path(),&trust,NOW),
        Err(PinError::Release(ReleaseError::RollbackRefused))));
}
#[test]
fn signed_release_expiry_rejected(){
    let (dir,trust)=setup();
    assert!(matches!(verify_root_staged(dir.path(),&trust,NOW+1000),
        Err(PinError::Release(ReleaseError::Expired))));
}
#[test]
fn later_preview_denies_expired_or_not_yet_valid_signed_package(){
    let (dir,trust)=setup();
    let pinned=verify_root_staged(dir.path(),&trust,NOW).unwrap();
    assert!(pinned.valid_at(NOW).is_ok());
    assert!(pinned.valid_at(NOW+499).is_ok());
    assert!(matches!(pinned.valid_at(NOW+500),
        Err(PinError::Release(ReleaseError::Expired))));
    assert!(matches!(pinned.valid_at(NOW-61),
        Err(PinError::Release(ReleaseError::NotYetValid))));
    assert!(pinned.stable().is_ok());
}
#[test]
fn symlink_node_refused_before_read(){
    let (dir,trust)=setup();
    fs::remove_file(dir.path().join("node")).unwrap();
    symlink("manifest",dir.path().join("node")).unwrap();
    assert!(verify_root_staged(dir.path(),&trust,NOW).is_err());
}
#[test]
fn hardlinked_node_refused_even_if_owned_by_root(){
    let (dir,trust)=setup();
    fs::hard_link(dir.path().join("node"),dir.path().join("linked")).unwrap();
    assert!(matches!(verify_root_staged(dir.path(),&trust,NOW),
        Err(PinError::BadOwnerOrMode)));
}
#[test]
fn wrong_mode_refused_before_verification(){
    let (dir,trust)=setup();
    fs::set_permissions(dir.path().join("node"),Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(verify_root_staged(dir.path(),&trust,NOW),
        Err(PinError::BadOwnerOrMode)));
}
#[test]
fn symlink_directory_refused(){
    let (dir,trust)=setup();
    let parent=tempfile::tempdir().unwrap();
    let link=parent.path().join("spoof");
    symlink(dir.path(),&link).unwrap();
    assert!(verify_root_staged(&link,&trust,NOW).is_err());
}
#[test]
fn noncanonical_manifest_trailing_bytes_refused(){
    let (dir,trust)=setup();
    let manifest=dir.path().join("manifest");
    fs::set_permissions(&manifest,Permissions::from_mode(0o600)).unwrap();
    let mut b=fs::read(&manifest).unwrap();
    b.push(0);
    write_mode(&manifest,&b,0o400);
    assert!(matches!(verify_root_staged(dir.path(),&trust,NOW),
        Err(PinError::BadManifest)));
}
#[test]
fn canonical_parser_rejects_missing_schema(){
    assert!(decode_b1a_wire(b"Akula Node B1a signed release manifest v1\0").is_err());
}
#[test]
fn in_place_modified_pinned_inode_does_not_remain_stable(){
    let (dir,trust)=setup();
    let v=verify_root_staged(dir.path(),&trust,NOW).unwrap();
    let node=dir.path().join("node");
    fs::set_permissions(&node,Permissions::from_mode(0o600)).unwrap();
    fs::write(&node,b"changed object").unwrap();
    fs::set_permissions(&node,Permissions::from_mode(0o400)).unwrap();
    assert!(matches!(v.stable(),Err(PinError::ChangedInode)));
}
