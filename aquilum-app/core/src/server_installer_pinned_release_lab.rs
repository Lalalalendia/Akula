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
    valid_from_unix:u64,
    valid_until_unix:u64,
    pub receipt:VerifiedPackageReceipt,
    verified_publisher_key:[u8;32],
    verified_publisher_id:String,
}
impl RootVerifiedFd{
    /// The exact signer used for B1a validation of this pinned FD; not a
    /// second, caller-supplied identity disconnected from that validation.
    #[cfg(feature = "server-root-b0v3-gate-lab")]
    pub fn verified_publisher(&self)->(&str,[u8;32]){
        (&self.verified_publisher_id,self.verified_publisher_key)
    }
    pub fn stable(&self)->Result<(),PinError>{
        if !same_object(&self.meta,&self.file.metadata()?){
            return Err(PinError::ChangedInode);
        }
        Ok(())
    }
    /// B1a time window must still hold at every new Preview admission.
    /// FD metadata alone cannot prove a previously verified release is fresh.
    /// This is NOT owner consent or authorization to install.
    pub fn valid_at(&self,now_unix:u64)->Result<(),PinError>{
        self.stable()?;
        if now_unix<self.valid_from_unix {
            return Err(PinError::Release(ReleaseError::NotYetValid));
        }
        if now_unix>=self.valid_until_unix {
            return Err(PinError::Release(ReleaseError::Expired));
        }
        Ok(())
    }

    /// B1c0 isolated test-only bridge: stream bytes from the SAME previously
    /// authenticated inode without trusting any path re-open or client data.
    /// Rehash the exact byte length before the caller may commit a fixed file.
    /// The caller must hold its own protected transaction lock and cleanly
    /// handle a partial destination after any failure.
    #[cfg(feature = "server-bootstrap-b1c0-fixed-node-lab")]
    pub(crate) fn copy_rechecked_bytes(
        &self, destination: &mut File, now_unix: u64,
    ) -> Result<(), PinError> {
        use ring::{constant_time, digest};
        use std::io::Write;
        use std::os::unix::fs::FileExt;
        self.valid_at(now_unix)?;
        let mut hash = digest::Context::new(&digest::SHA256);
        let mut offset = 0u64;
        let mut buf = [0u8; 32 * 1024];
        while offset < self.receipt.byte_length {
            let remain = self.receipt.byte_length - offset;
            let want = usize::try_from(remain.min(buf.len() as u64))
                .map_err(|_| PinError::Release(ReleaseError::SizeMismatch))?;
            let n = self.file.read_at(&mut buf[..want], offset)?;
            if n == 0 {
                return Err(PinError::Release(ReleaseError::SizeMismatch));
            }
            destination.write_all(&buf[..n])?;
            hash.update(&buf[..n]);
            offset = offset.checked_add(n as u64)
                .ok_or(PinError::Release(ReleaseError::SizeMismatch))?;
        }
        let mut extra = [0u8; 1];
        if self.file.read_at(&mut extra, offset)? != 0 {
            return Err(PinError::Release(ReleaseError::SizeMismatch));
        }
        if constant_time::verify_slices_are_equal(
            hash.finish().as_ref(), &self.receipt.package_sha256,
        ).is_err() {
            return Err(PinError::Release(ReleaseError::PackageDigestMismatch));
        }
        self.valid_at(now_unix)?;
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
    verify_root_staged_dir(&dir, trust, now_unix)
}

mod verify_dir;
pub(crate) use verify_dir::verify_root_staged_dir;
