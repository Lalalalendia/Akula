//! Descriptor-scoped signed B1a release admission in root-owned stage.
use super::*;
/// Verify a root-owned, O_NOFOLLOW-opened directory FD. Callers that
/// need trusted ancestors must open them component-by-component before
/// supplying this FD; the staged Node path itself grants no authority.
pub(crate) fn verify_root_staged_dir(
    dir: &File, trust: &RootPinnedReleaseTrust, now_unix: u64,
)->Result<RootVerifiedFd,PinError>{
    if rustix::process::geteuid().as_raw()!=0 {
        return Err(PinError::BadOwnerOrMode);
    }
    let d=dir.metadata()?;
    if !d.is_dir()||d.uid()!=0||d.nlink()==0||
        d.permissions().mode()&0o7777!=0o700
    {
        return Err(PinError::BadOwnerOrMode);
    }
    if trust.trusted_signer.ed25519_public==[0;32]||
       trust.approved_package_sha256==[0;32]||
       trust.approved_manifest_fingerprint==[0;32]||
       trust.approved_release_sequence==0{
        return Err(PinError::BadTrustAnchor);
    }
    let (mut mf,_)=open_fixed(dir,"manifest",0o400,MAX_MANIFEST as u64)?;
    let mut raw=Vec::new();
    mf.read_to_end(&mut raw)?;
    let manifest=decode_b1a_wire(&raw)?;
    let (mut sf,_)=open_fixed(dir,"signature",0o400,64)?;
    let mut sig=[0u8;64];
    sf.read_exact(&mut sig)?;
    let auth=verify_signed_release(
        &SignedRelease{manifest,signature:sig},
        &trust.trusted_signer,trust.previous_floor,now_unix)?;
    let floor=auth.release_floor_candidate();
    let valid_from_unix=auth.manifest().not_before_unix;
    let valid_until_unix=auth.manifest().expires_unix;
    if floor.sequence!=trust.approved_release_sequence||
        floor.signed_manifest_fingerprint!=trust.approved_manifest_fingerprint||
        auth.manifest().package_sha256!=trust.approved_package_sha256{
        return Err(PinError::WrongPolicyScope);
    }
    let (mut file,before)=open_fixed(dir,"node",0o400,MAX_NODE_PACKAGE_BYTES)?;
    if before.len()!=auth.manifest().package_size{
        return Err(PinError::Release(ReleaseError::SizeMismatch));
    }
    let receipt=auth.verify_package_stream(&mut file)?;
    let after=file.metadata()?;
    if !same_object(&before,&after){
        return Err(PinError::ChangedInode);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(RootVerifiedFd{
        file,meta:after,receipt,valid_from_unix,valid_until_unix,
        verified_publisher_key:trust.trusted_signer.ed25519_public,
        verified_publisher_id:trust.trusted_signer.signer_id.clone(),
    })
}
