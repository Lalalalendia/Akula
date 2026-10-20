//! B1b3c: Linux root-owned FD-pinned B1a verification. Never installs,
//! executes, activates, modifies a file or grants owner consent.
use crate::server_release_lab::{
    verify_signed_release, ReleaseError, ReleaseFloor, ReleaseManifestV1,
    SignedRelease, TargetPlatform, TrustedReleaseSigner, VerifiedPackageReceipt,
    Version, MAX_NODE_PACKAGE_BYTES,
};
use rustix::fs::{open,openat,Mode,OFlags};
use std::fs::{File,Metadata};
use std::io::{Read,Seek,SeekFrom};
use std::os::unix::fs::{MetadataExt,PermissionsExt};
use std::path::Path;

const DOMAIN:&[u8]=b"Akula Node B1a signed release manifest v1\0";
const MAX_MANIFEST:usize=256;
#[derive(Debug)]
pub enum PinError{
    Io(std::io::Error),BadOwnerOrMode,BadManifest,BadTrustAnchor,
    Release(ReleaseError),WrongPolicyScope,ChangedInode,
}
impl From<std::io::Error> for PinError {
    fn from(e:std::io::Error)->Self{Self::Io(e)}
}
impl From<ReleaseError> for PinError{
    fn from(e:ReleaseError)->Self{Self::Release(e)}
}
fn os(e:rustix::io::Errno)->PinError{PinError::Io(e.into())}
fn take<const N:usize>(b:&[u8],p:&mut usize)->Result<[u8;N],PinError>{
    let end=p.checked_add(N).ok_or(PinError::BadManifest)?;
    let v=b.get(*p..end).ok_or(PinError::BadManifest)?;
    *p=end;
    v.try_into().map_err(|_|PinError::BadManifest)
}
fn u32be(b:&[u8],p:&mut usize)->Result<u32,PinError>{
    Ok(u32::from_be_bytes(take::<4>(b,p)?))
}
fn u64be(b:&[u8],p:&mut usize)->Result<u64,PinError>{
    Ok(u64::from_be_bytes(take::<8>(b,p)?))
}
/// Parse only byte-for-byte canonical B1a binary manifests.
pub fn decode_b1a_wire(b:&[u8])->Result<ReleaseManifestV1,PinError>{
    if b.len()>MAX_MANIFEST||!b.starts_with(DOMAIN){return Err(PinError::BadManifest);}
    let mut p=DOMAIN.len();
    if take::<1>(b,&mut p)?!=[1]||take::<1>(b,&mut p)?!=[1]{
        return Err(PinError::BadManifest);
    }
    let version=Version{major:u32be(b,&mut p)?,minor:u32be(b,&mut p)?,
        patch:u32be(b,&mut p)?};
    let release_sequence=u64be(b,&mut p)?;
    let package_size=u64be(b,&mut p)?;
    let not_before_unix=u64be(b,&mut p)?;
    let expires_unix=u64be(b,&mut p)?;
    let package_sha256=take::<32>(b,&mut p)?;
    let n=take::<1>(b,&mut p)?[0] as usize;
    let end=p.checked_add(n).ok_or(PinError::BadManifest)?;
    let signer=b.get(p..end).ok_or(PinError::BadManifest)?;
    p=end;
    let signer_id=String::from_utf8(signer.to_vec()).map_err(|_|PinError::BadManifest)?;
    let m=ReleaseManifestV1{
        platform:TargetPlatform::LinuxX86_64,version,release_sequence,
        package_size,not_before_unix,expires_unix,package_sha256,signer_id,
    };
    if p!=b.len()||m.canonical_signed_bytes()!=b{
        return Err(PinError::BadManifest);
    }
    Ok(m)
}
/// Construct ONLY from independently provisioned OS-owned trust state.
pub struct RootPinnedReleaseTrust {
    pub trusted_signer:TrustedReleaseSigner,
    pub previous_floor:ReleaseFloor,
    pub approved_manifest_fingerprint:[u8;32],
    pub approved_package_sha256:[u8;32],
    pub approved_release_sequence:u64,
}
fn file_ok(m:&Metadata,mode:u32)->bool{
    m.is_file() && m.uid()==0 && m.nlink()==1 &&
        m.permissions().mode()&0o7777==mode
}
fn open_fixed(dir:&File,name:&str,mode:u32,max:u64)->Result<(File,Metadata),PinError>{
    let f=File::from(openat(dir,name,
        OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::CLOEXEC,
        Mode::empty()).map_err(os)?);
    let m=f.metadata()?;
    if !file_ok(&m,mode)||m.len()==0||m.len()>max{
        return Err(PinError::BadOwnerOrMode);
    }
    Ok((f,m))
}
fn same_object(a:&Metadata,b:&Metadata)->bool{
    a.dev()==b.dev()&&a.ino()==b.ino()&&a.len()==b.len()
        &&a.mtime()==b.mtime()&&a.mtime_nsec()==b.mtime_nsec()
        &&a.ctime()==b.ctime()&&a.ctime_nsec()==b.ctime_nsec()
}
/// Keeps the already-verified descriptor open. It is intentionally not an
/// executable handle and gives no install or write authority.
pub struct RootVerifiedFd{
    file:File,
    meta:Metadata,
    pub receipt:VerifiedPackageReceipt,
}
impl RootVerifiedFd{
    pub fn stable(&self)->Result<(),PinError>{
        if !same_object(&self.meta,&self.file.metadata()?){
            return Err(PinError::ChangedInode);
        }
        Ok(())
    }
    pub fn inode_identity(&self)->(u64,u64){
        (self.meta.dev(),self.meta.ino())
    }
}
/// The caller MUST pass a compile-time fixed root-controlled directory,
/// and a trust anchor loaded independently of the package or IPC client.
pub fn verify_root_staged(
    dir_path:&Path,trust:&RootPinnedReleaseTrust,now_unix:u64,
)->Result<RootVerifiedFd,PinError>{
    if rustix::process::geteuid().as_raw()!=0 {
        return Err(PinError::BadOwnerOrMode);
    }
    let dir=File::from(open(dir_path,
        OFlags::RDONLY|OFlags::DIRECTORY|OFlags::NOFOLLOW|OFlags::CLOEXEC,
        Mode::empty()).map_err(os)?);
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
    let (mut mf,_)=open_fixed(&dir,"manifest",0o400,MAX_MANIFEST as u64)?;
    let mut raw=Vec::new();
    mf.read_to_end(&mut raw)?;
    let manifest=decode_b1a_wire(&raw)?;
    let (mut sf,_)=open_fixed(&dir,"signature",0o400,64)?;
    let mut sig=[0u8;64];
    sf.read_exact(&mut sig)?;
    let auth=verify_signed_release(
        &SignedRelease{manifest,signature:sig},
        &trust.trusted_signer,trust.previous_floor,now_unix)?;
    let floor=auth.release_floor_candidate();
    if floor.sequence!=trust.approved_release_sequence||
        floor.signed_manifest_fingerprint!=trust.approved_manifest_fingerprint||
        auth.manifest().package_sha256!=trust.approved_package_sha256{
        return Err(PinError::WrongPolicyScope);
    }
    let (mut file,before)=open_fixed(&dir,"node",0o400,MAX_NODE_PACKAGE_BYTES)?;
    if before.len()!=auth.manifest().package_size{
        return Err(PinError::Release(ReleaseError::SizeMismatch));
    }
    let receipt=auth.verify_package_stream(&mut file)?;
    let after=file.metadata()?;
    if !same_object(&before,&after){
        return Err(PinError::ChangedInode);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(RootVerifiedFd{file,meta:after,receipt})
}
