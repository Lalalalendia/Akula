//! Isolated P0e OS name-exchange lab. TEST ONLY; never a production writer.
//! Pinned-handle exchange is not inode CAS and never confirms a sync baseline.
//! Old bytes remain at the staging name. Never rollback/cleanup on uncertainty.
use super::{OperationId,PinnedFileIdentity};
use std::fs::File;

#[derive(Debug,PartialEq,Eq)]
enum ProbeError { Invalid, Unsupported, Os(String), Ambiguous }

#[cfg(any(target_os="linux",target_os="macos"))]
mod unix {
    use super::*;
    use rustix::fs::{openat,OFlags,Mode};
    use std::io::{Read,Seek,Write};
    use std::os::unix::fs::MetadataExt;
    const MAX: usize=32*1024*1024;

    fn file_id(f:&File)->Result<PinnedFileIdentity,ProbeError>{
        let m=f.metadata().map_err(|e|ProbeError::Os(e.to_string()))?;
        if !m.is_file()||m.nlink()!=1||m.ino()==0||m.dev()==0{return Err(ProbeError::Invalid);}
        Ok(PinnedFileIdentity{device:m.dev(),file:m.ino()})
    }
    fn dir_id(f:&File)->Result<PinnedFileIdentity,ProbeError>{
        let m=f.metadata().map_err(|e|ProbeError::Os(e.to_string()))?;
        if !m.is_dir()||m.ino()==0||m.dev()==0{return Err(ProbeError::Invalid);}
        Ok(PinnedFileIdentity{device:m.dev(),file:m.ino()})
    }
    fn read_child(parent:&File,name:&str,limit:usize)
      ->Result<(PinnedFileIdentity,Vec<u8>),ProbeError>{
        let f:File=openat(parent,name,OFlags::RDONLY|OFlags::NOFOLLOW
            |OFlags::NONBLOCK|OFlags::CLOEXEC,Mode::empty())
            .map(File::from).map_err(|e|ProbeError::Os(e.to_string()))?;
        let id=file_id(&f)?;
        let mut data=Vec::new();
        (&f).take(limit as u64+1).read_to_end(&mut data)
            .map_err(|e|ProbeError::Os(e.to_string()))?;
        if data.len()>limit||file_id(&f)?!=id{return Err(ProbeError::Invalid);}
        Ok((id,data))
    }
    fn swap(parent:&File,stage:&str,dest:&str)->Result<(),ProbeError>{
        #[cfg(target_os="linux")]
        {
            use rustix::fs::{renameat_with,RenameFlags};
            renameat_with(parent,stage,parent,dest,RenameFlags::EXCHANGE)
                .map_err(|e|ProbeError::Os(e.to_string()))
        }
        #[cfg(target_os="macos")]
        {
            use std::ffi::CString;
            use std::os::fd::AsRawFd;
            #[link(name="System")]
            extern "C" {
                fn renameatx_np(a:i32,ap:*const std::ffi::c_char,
                                b:i32,bp:*const std::ffi::c_char,flags:u32)->i32;
            }
            let a=CString::new(stage).map_err(|_|ProbeError::Invalid)?;
            let b=CString::new(dest).map_err(|_|ProbeError::Invalid)?;
            // XNU RENAME_SWAP=0x2. Unsupported filesystems fail closed.
            let n=unsafe{renameatx_np(parent.as_raw_fd(),a.as_ptr(),
                                     parent.as_raw_fd(),b.as_ptr(),0x2)};
            if n==0{Ok(())}else{Err(ProbeError::Os(
                std::io::Error::last_os_error().to_string()))}
        }
    }
    pub(super) fn run(parent:&File,leaf:&str,op:OperationId,
        expected_id:PinnedFileIdentity,original:&[u8],updated:&[u8],
        before:impl FnOnce(),after:impl FnOnce())
        ->Result<String,ProbeError>{
        if leaf.is_empty()||leaf=="."||leaf==".."||leaf.starts_with(".akula-sync-")
            ||leaf.chars().any(|c|c=='/'||c=='\\'||c==':'||c=='\0'||c.is_control())
            ||original.len()>MAX||updated.len()>MAX{return Err(ProbeError::Invalid);}
        let parent_before=dir_id(parent)?;
        if read_child(parent,leaf,original.len())?!=(expected_id,original.to_vec()){
            return Err(ProbeError::Invalid);
        }
        let name=format!(".akula-sync-test-swap-{op}");
        let mut stage:File=openat(parent,name.as_str(),OFlags::RDWR|OFlags::CREATE
            |OFlags::EXCL|OFlags::NOFOLLOW|OFlags::CLOEXEC,
            Mode::RUSR|Mode::WUSR)
            .map(File::from).map_err(|e|ProbeError::Os(e.to_string()))?;
        stage.write_all(updated).map_err(|e|ProbeError::Os(e.to_string()))?;
        stage.sync_all().map_err(|e|ProbeError::Os(e.to_string()))?;
        let new_id=file_id(&stage)?;
        stage.rewind().map_err(|e|ProbeError::Os(e.to_string()))?;
        let mut check=Vec::new();
        (&mut stage).take(updated.len() as u64+1).read_to_end(&mut check)
            .map_err(|e|ProbeError::Os(e.to_string()))?;
        if check!=updated{return Err(ProbeError::Invalid);}
        before();
        if dir_id(parent)?!=parent_before
            ||read_child(parent,leaf,original.len())?!=(expected_id,original.to_vec())
        {return Err(ProbeError::Invalid);}
        // Atomic name exchange, NOT an atomic check of expected_id.
        swap(parent,&name,leaf)?;
        after(); // simulate SIGKILL/lost acknowledgement before durable receipt
        if read_child(parent,leaf,updated.len())
              .map_err(|_|ProbeError::Ambiguous)?!=(new_id,updated.to_vec())
            ||read_child(parent,&name,original.len())
              .map_err(|_|ProbeError::Ambiguous)?!=(expected_id,original.to_vec())
            ||dir_id(parent).map_err(|_|ProbeError::Ambiguous)?!=parent_before
        {return Err(ProbeError::Ambiguous);}
        parent.sync_all().map_err(|_|ProbeError::Ambiguous)?;
        Ok(name) // informational only, no native receipt or baseline
    }
}
#[cfg(any(target_os="linux",target_os="macos"))]
use unix::run;

#[cfg(not(any(target_os="linux",target_os="macos")))]
fn run(_parent:&File,_leaf:&str,_op:OperationId,_id:PinnedFileIdentity,
    _original:&[u8],_updated:&[u8],_before:impl FnOnce(),_after:impl FnOnce())
    ->Result<String,ProbeError>{Err(ProbeError::Unsupported)}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(any(target_os="linux",target_os="macos"))]
    fn fixture()->(tempfile::TempDir,File,PinnedFileIdentity){
        use std::os::unix::fs::MetadataExt;
        let dir=tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.md"),b"old confirmed").unwrap();
        let meta=fs::metadata(dir.path().join("file.md")).unwrap();
        let id=PinnedFileIdentity{device:meta.dev(),file:meta.ino()};
        let parent=File::open(dir.path()).unwrap();
        (dir,parent,id)
    }
    #[cfg(any(target_os="linux",target_os="macos"))]
    #[test]
    fn exchange_retains_both_versions_under_distinct_names(){
        let (d,p,id)=fixture();
        let stage=run(&p,"file.md",OperationId::new(),id,
            b"old confirmed",b"new verified",||{},||{}).unwrap();
        assert_eq!(fs::read(d.path().join("file.md")).unwrap(),b"new verified");
        assert_eq!(fs::read(d.path().join(stage)).unwrap(),b"old confirmed");
    }
    #[cfg(any(target_os="linux",target_os="macos"))]
    #[test]
    fn same_hash_but_foreign_inode_refuses_before_exchange(){
        let (d,p,id)=fixture();
        let file=d.path().join("file.md");
        let other=d.path().join("other.md");
        let result=run(&p,"file.md",OperationId::new(),id,
            b"old confirmed",b"new verified",||{
                fs::rename(&file,&other).unwrap();
                fs::write(&file,b"old confirmed").unwrap();
            },||{});
        assert_eq!(result,Err(ProbeError::Invalid));
        assert_eq!(fs::read(file).unwrap(),b"old confirmed");
    }
    #[cfg(any(target_os="linux",target_os="macos"))]
    #[test]
    fn interrupted_after_exchange_leaves_hold_evidence_not_safe_replay(){
        let (d,p,id)=fixture();
        let op=OperationId::new();
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
            let _=run(&p,"file.md",op,id,b"old confirmed",b"new verified",
                ||{},||panic!("simulate lost acknowledgment"));
        }));
        assert!(result.is_err());
        assert_eq!(fs::read(d.path().join("file.md")).unwrap(),b"new verified");
        assert_eq!(fs::read(d.path().join(format!(".akula-sync-test-swap-{op}")))
            .unwrap(),b"old confirmed");
    }
    #[cfg(any(target_os="linux",target_os="macos"))]
    #[test]
    fn stage_collision_must_not_replace_user_file(){
        let (d,p,id)=fixture();
        let op=OperationId::new();
        fs::write(d.path().join(format!(".akula-sync-test-swap-{op}")),
            b"other worker").unwrap();
        assert!(run(&p,"file.md",op,id,b"old confirmed",b"new verified",
            ||{},||{}).is_err());
        assert_eq!(fs::read(d.path().join("file.md")).unwrap(),b"old confirmed");
    }
    #[cfg(not(any(target_os="linux",target_os="macos")))]
    #[test]
    fn windows_must_fail_closed_without_native_durable_receipt(){
        let d=tempfile::tempdir().unwrap();
        let p=File::create(d.path().join("dummy")).unwrap();
        assert_eq!(run(&p,"file.md",OperationId::new(),
            PinnedFileIdentity{device:1,file:1},b"old",b"new",||{},||{}),
            Err(ProbeError::Unsupported));
    }
}
