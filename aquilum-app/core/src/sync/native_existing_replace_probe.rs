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
            // An open stage handle does not pin its pathname. Reject both
            // same-byte foreign inodes and changed bytes before publication.
            ||read_child(parent,&name,updated.len())?!=(new_id,updated.to_vec())
        {return Err(ProbeError::Invalid);}
        // Atomic name exchange, NOT an atomic check of either expected ID.
        // Even a syscall error is uncertain once the operation was attempted.
        swap(parent,&name,leaf).map_err(|_|ProbeError::Ambiguous)?;
        after(); // Hook for distinct panic/unwind AND real process-kill tests.
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
    fn substituted_stage_name_refuses_before_exchange(){
        for foreign in [&b"new verified"[..], &b"foreign data"[..]] {
            let (d,p,id)=fixture();
            let op=OperationId::new();
            let stage=d.path().join(format!(".akula-sync-test-swap-{op}"));
            let kept=d.path().join("parked-stage");
            let result=run(&p,"file.md",op,id,b"old confirmed",b"new verified",||{
                fs::rename(&stage,&kept).unwrap();
                fs::write(&stage,foreign).unwrap();
            },||{});
            assert_eq!(result,Err(ProbeError::Invalid));
            assert_eq!(fs::read(d.path().join("file.md")).unwrap(),b"old confirmed");
            assert_eq!(fs::read(&kept).unwrap(),b"new verified");
            assert_eq!(fs::read(&stage).unwrap(),foreign);
        }
    }
    #[cfg(any(target_os="linux",target_os="macos"))]
    #[test]
    fn changed_stage_bytes_refuse_before_exchange(){
        let (d,p,id)=fixture();
        let op=OperationId::new();
        let stage=d.path().join(format!(".akula-sync-test-swap-{op}"));
        let result=run(&p,"file.md",op,id,b"old confirmed",b"new verified",||{
            fs::write(&stage,b"bad verified").unwrap();
        },||{});
        assert_eq!(result,Err(ProbeError::Invalid));
        assert_eq!(fs::read(d.path().join("file.md")).unwrap(),b"old confirmed");
        assert_eq!(fs::read(&stage).unwrap(),b"bad verified");
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

    #[cfg(any(target_os="linux",target_os="macos"))]
    mod crash {
        use super::{fixture, run};
        use crate::sync::{DocumentId, JournalAction, JournalError, JournalPhase,
            OperationId, OperationIntent, PinnedFileIdentity, RecoveryAction,
            RemoteGuard, RevisionId, SqliteSyncJournal, SyncRelativePath, VaultId};
        use crate::sync::protection_store::{BoundProtectedOriginal, ProtectedOriginal};
        use std::fs::{self, File, OpenOptions};
        use std::io::Write;
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::process::ExitStatusExt;
        use std::path::Path;
        use std::process::{Child, Command, Stdio};
        use std::time::{Duration, Instant};

        const CHILD: &str = "sync::native_existing_replace_probe::tests::crash::native_exchange_crash_child";
        const OLD: &[u8] = b"old confirmed";
        const NEW: &[u8] = b"new verified";

        // Never leave a stopped child or a zombie if a parent assertion fails.
        struct ChildGuard(Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        fn protected(root: &Path, op: OperationId, file: PinnedFileIdentity)
            -> BoundProtectedOriginal
        {
            let parent = fs::metadata(root).unwrap();
            BoundProtectedOriginal {
                original: ProtectedOriginal {
                    operation_id: op,
                    path: SyncRelativePath::parse("file.md").unwrap(),
                    content_hash: blake3::hash(OLD).to_hex().to_string(),
                    bytes: OLD.to_vec(),
                },
                root: PinnedFileIdentity { device: parent.dev(), file: parent.ino() },
                file,
            }
        }

        fn seed_journal(root: &Path) -> OperationIntent {
            let mut journal = SqliteSyncJournal::open(&root.join("journal.sqlite")).unwrap();
            let mut previous = OperationIntent {
                operation_id: OperationId::new(), vault_id: VaultId::new(),
                document_id: DocumentId::new(), profile_id: "crash-fixture".into(),
                revision_id: RevisionId::new(), action: JournalAction::UploadImmutable,
                path: SyncRelativePath::parse("file.md").unwrap(),
                remote_object_id: "fixture/old".into(),
                remote_guard: Some(RemoteGuard::Absent),
                expected_content_hash: blake3::hash(OLD).to_hex().to_string(),
                expected_baseline: None,
            };
            journal.begin(previous.clone()).unwrap();
            journal.mark_staged(previous.operation_id, OLD).unwrap();
            journal.mark_applied(previous.operation_id).unwrap();
            journal.mark_verified(previous.operation_id, OLD, "fixture-provider").unwrap();
            journal.commit(previous.operation_id).unwrap();
            // Synthetic baseline fixture, not a real remote verification.
            previous.expected_baseline = Some(previous.revision_id);
            previous.operation_id = OperationId::new();
            previous.revision_id = RevisionId::new();
            previous.action = JournalAction::StageDownload;
            previous.remote_object_id = "fixture/new".into();
            previous.remote_guard = None;
            previous.expected_content_hash = blake3::hash(NEW).to_hex().to_string();
            journal.begin(previous.clone()).unwrap();
            journal.mark_staged(previous.operation_id, NEW).unwrap();
            previous
        }

        fn stop_at(root: &Path, mode: &str) -> ! {
            let mut ready = OpenOptions::new().write(true).create_new(true)
                .open(root.join("child-ready")).unwrap();
            ready.write_all(mode.as_bytes()).unwrap();
            ready.sync_all().unwrap();
            loop { std::thread::sleep(Duration::from_millis(10)); }
        }

        #[test]
        fn native_exchange_crash_child() {
            let Some(mode) = std::env::var("AKULA_NATIVE_EXCHANGE_CRASH_MODE").ok()
            else { return; };
            assert!(matches!(mode.as_str(), "before_exchange" | "after_exchange" | "after_parent_sync"));
            let root = std::path::PathBuf::from(
                std::env::var_os("AKULA_NATIVE_EXCHANGE_CRASH_ROOT").unwrap());
            let operation: OperationId = std::env::var("AKULA_NATIVE_EXCHANGE_CRASH_OP")
                .unwrap().parse().unwrap();
            let id = PinnedFileIdentity {
                device: std::env::var("AKULA_NATIVE_EXCHANGE_CRASH_DEVICE").unwrap().parse().unwrap(),
                file: std::env::var("AKULA_NATIVE_EXCHANGE_CRASH_FILE").unwrap().parse().unwrap(),
            };
            let mut journal = SqliteSyncJournal::open(&root.join("journal.sqlite")).unwrap();
            let intent = journal.get(operation).unwrap().unwrap().intent;
            let original = protected(&root, operation, id);
            journal.record_existing_replace_attempt_once(&intent, &original).unwrap();
            let parent = File::open(&root).unwrap();
            run(&parent, "file.md", operation, id, OLD, NEW,
                || { if mode == "before_exchange" { stop_at(&root, &mode); } },
                || { if mode == "after_exchange" { stop_at(&root, &mode); } },
            ).unwrap();
            assert_eq!(mode, "after_parent_sync");
            stop_at(&root, &mode);
        }

        #[test]
        fn sigkill_at_native_exchange_boundaries_retains_claim_and_both_versions() {
            for mode in ["before_exchange", "after_exchange", "after_parent_sync"] {
                let (dir, parent, id) = fixture();
                let intent = seed_journal(dir.path());
                let original = protected(dir.path(), intent.operation_id, id);
                let baseline = {
                    let journal = SqliteSyncJournal::open(&dir.path().join("journal.sqlite")).unwrap();
                    journal.baseline(intent.vault_id, intent.document_id).unwrap()
                };
                drop(parent); // Child gets new descriptors; no parent DB handles remain.
                let log_path = dir.path().join("child.log");
                let log = File::create(&log_path).unwrap();
                let mut child = ChildGuard(Command::new(std::env::current_exe().unwrap())
                    .arg("--exact").arg(CHILD).arg("--nocapture")
                    .env("AKULA_NATIVE_EXCHANGE_CRASH_MODE", mode)
                    .env("AKULA_NATIVE_EXCHANGE_CRASH_ROOT", dir.path())
                    .env("AKULA_NATIVE_EXCHANGE_CRASH_OP", intent.operation_id.to_string())
                    .env("AKULA_NATIVE_EXCHANGE_CRASH_DEVICE", id.device.to_string())
                    .env("AKULA_NATIVE_EXCHANGE_CRASH_FILE", id.file.to_string())
                    .stdout(Stdio::from(log.try_clone().unwrap()))
                    .stderr(Stdio::from(log))
                    .spawn().unwrap());
                let deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    if fs::read(dir.path().join("child-ready")).ok().as_deref()
                        == Some(mode.as_bytes()) { break; }
                    if let Some(status) = child.0.try_wait().unwrap() {
                        panic!("child exited before {mode}: {status}; {}",
                            fs::read_to_string(&log_path).unwrap_or_default());
                    }
                    assert!(Instant::now() < deadline, "timed out at {mode}: {}",
                        fs::read_to_string(&log_path).unwrap_or_default());
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(child.0.try_wait().unwrap().is_none());
                child.0.kill().unwrap();
                let status = child.0.wait().unwrap();
                assert_eq!(status.signal(), Some(9), "must be real SIGKILL, not panic/exit");
                let stage = dir.path().join(format!(".akula-sync-test-swap-{}", intent.operation_id));
                let target = dir.path().join("file.md");
                let before = mode == "before_exchange";
                assert_eq!(fs::read(&target).unwrap(), if before { OLD } else { NEW });
                assert_eq!(fs::read(&stage).unwrap(), if before { NEW } else { OLD });
                let old_meta = fs::metadata(if before { &target } else { &stage }).unwrap();
                assert_eq!((old_meta.dev(), old_meta.ino()), (id.device, id.file));
                let mut recovered = SqliteSyncJournal::open(&dir.path().join("journal.sqlite")).unwrap();
                let claim = recovered.read_existing_replace_attempt(intent.operation_id)
                    .unwrap().expect("committed attempt must survive child SIGKILL");
                assert_eq!(claim.original_file, id);
                assert_eq!(recovered.baseline(intent.vault_id, intent.document_id).unwrap(), baseline);
                assert_eq!(recovered.get(intent.operation_id).unwrap().unwrap().phase, JournalPhase::Staged);
                assert!(matches!(recovered.record_existing_replace_attempt_once(&intent, &original),
                    Err(JournalError::ExistingReplaceAttemptAlreadyRecorded)));
                assert!(matches!(recovered.mark_applied(intent.operation_id),
                    Err(JournalError::LocalDownloadNeedsNativeReceipt)));
                assert!(recovered.recovery_queue().unwrap().iter().any(|item|
                    item.record.intent.operation_id == intent.operation_id
                        && item.action == RecoveryAction::InspectBeforeRetry));
                eprintln!("AKULA_NATIVE_EXCHANGE_SIGKILL boundary={mode} signal=9 journal=staged baseline=unchanged replay=denied");
            }
        }
    }
}
