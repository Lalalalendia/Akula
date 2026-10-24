//! Receiver-local durable-enough-for-process-restart Inbox, NOT a signed
//! delivery ACK or proof of OS-owned Windows path custody.
//! The only authority here is a previously authenticated, explicitly requested
//! one-file QUIC reception. No sender-chosen names or Vault paths are accepted.

use akula_inbox_ledger_lab::{ExpectedTransfer, InboxLedger, LedgerError, LocalStatus, Transport};
use aquilum_core::p2p_inbox_lab::{InboxError, save_verified_stage_as_new};
#[cfg(windows)]
use aquilum_core::p2p_inbox_lab::{
    audit_pinned_windows_inbox_storage, create_pinned_windows_inbox_stage,
    mark_pinned_windows_stage_delete_on_close, pin_windows_inbox_sqlite_custody,
    publish_pinned_windows_inbox_object, verify_pinned_windows_inbox_object,
};
#[cfg(not(windows))]
use std::ffi::OsString;
#[cfg(not(windows))]
use std::fs::OpenOptions;
use std::{
    fs::{self, File},
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

// The Windows stage is removed by its originally opened file HANDLE, never
// by an ambient path that a same-user actor could rename and replace. On a
// failed receive, Drop still attempts deletion; successful receipts require
// a confirmed disposition before returning an ACK.
#[cfg(windows)]
struct PinnedStageCleanup(Option<File>);

#[cfg(windows)]
impl PinnedStageCleanup {
    fn finish(&mut self) -> Result<(), String> {
        if let Some(stage) = self.0.as_ref() {
            mark_pinned_windows_stage_delete_on_close(stage)
                .map_err(|error| format!("Windows pinned stage delete refused: {error:?}"))?;
            // NtSetInformationFile(FileDispositionInformation) permits only
            // closing the marked FileObject; never inspect it again.
            self.0.take();
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for PinnedStageCleanup {
    fn drop(&mut self) {
        if let Some(stage) = self.0.as_ref() {
            let _ = mark_pinned_windows_stage_delete_on_close(stage);
        }
    }
}

#[cfg(not(windows))]
struct RemoveOnDrop(PathBuf);

#[cfg(not(windows))]
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

// Negative, path-based Windows guard. This rejects stable junction/symlink
// ancestors, but does NOT establish handle-relative namespace/ACL custody.
#[cfg(windows)]
fn check_windows_directory_chain(root: &Path, create_missing: bool) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

    fn inspect(path: &Path) -> Result<bool, String> {
        match fs::symlink_metadata(path) {
            Ok(meta)
                if meta.is_dir() && meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 =>
            {
                Ok(true)
            }
            Ok(_) => Err(format!(
                "receiver Inbox directory is a reparse point or not a directory: {}",
                path.display()
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!("inspect receiver Inbox directory: {error}")),
        }
    }

    if !root.is_absolute() {
        return Err("receiver Inbox root must be absolute".into());
    }
    let mut absent = Vec::new();
    for ancestor in root.ancestors() {
        if !inspect(ancestor)? {
            if !create_missing {
                return Err("receiver Inbox directory disappeared or is missing".into());
            }
            absent.push(ancestor.to_path_buf());
        }
    }
    for directory in absent.into_iter().rev() {
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("create receiver Inbox directory: {error}")),
        }
        if !inspect(&directory)? {
            return Err("receiver Inbox directory disappeared after creation".into());
        }
    }
    // Recheck the entire chain, not just its final name. This remains a
    // point-in-time negative check; it cannot defeat hostile ancestor races.
    for ancestor in root.ancestors() {
        if !inspect(ancestor)? {
            return Err("receiver Inbox directory disappeared during inspection".into());
        }
    }
    Ok(())
}

/// Consistent local-only root that a future Akula Native Inbox can read.
/// Not taken from any QUIC frame, URL, remote label or provider metadata.
pub(super) fn default_root() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA is not available for this user")?;
    #[cfg(not(windows))]
    let base = {
        let selected: OsString = match std::env::var_os("XDG_DATA_HOME") {
            Some(value) => value,
            None => {
                let home = std::env::var_os("HOME").ok_or("HOME is not available")?;
                PathBuf::from(home).join(".local/share").into_os_string()
            }
        };
        PathBuf::from(selected)
    };
    if !base.is_absolute() {
        return Err("receiver app data base must be an absolute local path".into());
    }
    let root = base.join("Akula").join("received-inbox");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(&root)
            .map_err(|error| format!("create private receiver Inbox: {error}"))?;
        let metadata = fs::symlink_metadata(&root)
            .map_err(|error| format!("private receiver Inbox metadata: {error}"))?;
        if !metadata.is_dir() || (metadata.permissions().mode() & 0o077) != 0 {
            return Err("receiver Inbox root is not an exclusive owner-only directory".into());
        }
    }
    #[cfg(windows)]
    check_windows_directory_chain(&root, true)?;
    Ok(root)
}

/// Until the signed Offer UUID exists, duplicate content from one authenticated
/// sender to the same receiver intentionally maps to the same local object.
fn expected_transfer(
    size: u64,
    digest: [u8; 32],
    sender: [u8; 32],
    receiver: [u8; 32],
) -> ExpectedTransfer {
    let mut hash = blake3::Hasher::new_derive_key("Akula one-file LAN beta received content v1");
    hash.update(&sender);
    hash.update(&receiver);
    hash.update(&size.to_be_bytes());
    hash.update(&digest);
    let id_hash = hash.finalize();
    let mut id = [0u8; 16];
    id.copy_from_slice(&id_hash.as_bytes()[..16]);
    id[6] = (id[6] & 0x0f) | 0x80; // Deterministic UUID version 8
    id[8] = (id[8] & 0x3f) | 0x80;
    ExpectedTransfer {
        id: Uuid::from_bytes(id),
        sender_key: sender,
        recipient_key: receiver,
        source: Transport::P2p,
        bytes: size,
        blake3: digest,
    }
}

// The ledger owns intent validation and immutable receipt/replay semantics.
// A writer only publishes bytes in a locally selected, new-only destination.
fn persist_received_with_publisher<F>(
    pinned_stage: &mut File,
    inbox_root: &Path,
    size: u64,
    digest: [u8; 32],
    authenticated_sender: [u8; 32],
    local_receiver: [u8; 32],
    publisher: F,
) -> Result<ExpectedTransfer, String>
where
    F: FnOnce(&mut File, &Path, Uuid) -> Result<(), LedgerError>,
{
    let intent = expected_transfer(size, digest, authenticated_sender, local_receiver);
    let transfer_id = intent.id;
    let mut ledger = InboxLedger::open(inbox_root)
        .map_err(|error| format!("open receiver Inbox ledger: {error:?}"))?;
    let state = ledger
        .receive_with_writer(&intent, |destination| {
            publisher(pinned_stage, destination, transfer_id)
        })
        .map_err(|error| format!("Inbox rejected local object: {error:?}"))?;
    if state != LocalStatus::VerifiedLocal {
        return Err("receiver Inbox refused an unverified local object".into());
    }
    Ok(intent)
}

/// Legacy explicit-destination CLI retains the independent no-replace writer.
/// The installed/native Windows Inbox deliberately uses a separate, pinned NT
/// object publisher via persist_received_with_publisher.
pub(super) fn persist_received(
    pinned_stage: &mut File,
    inbox_root: &Path,
    size: u64,
    digest: [u8; 32],
    authenticated_sender: [u8; 32],
    local_receiver: [u8; 32],
) -> Result<ExpectedTransfer, String> {
    persist_received_with_publisher(
        pinned_stage,
        inbox_root,
        size,
        digest,
        authenticated_sender,
        local_receiver,
        |stage, destination, _| {
            save_verified_stage_as_new(stage, size, digest, destination)
                .map(|_| ())
                .map_err(|error| match error {
                    InboxError::Io(source) => LedgerError::Io(source),
                    InboxError::AlreadyExists => {
                        LedgerError::Io(io::Error::from(io::ErrorKind::AlreadyExists))
                    }
                    _ => LedgerError::InvalidLocalObject,
                })
        },
    )
}

/// The Native recipient has **no** user destination. Persist a verified
/// frame exclusively in the bounded, separately audited SQLite Inbox.
/// A private, new-only stage is removed on success and every failure.
pub(super) fn persist_received_payload(
    data: &[u8],
    inbox_root: &Path,
    size: u64,
    digest: [u8; 32],
    sender: [u8; 32],
    receiver: [u8; 32],
) -> Result<ExpectedTransfer, String> {
    if size == 0
        || size > aquilum_core::p2p_inbox_lab::MAX_INBOX_ITEM_BYTES
        || data.len() as u64 != size
        || digest == [0; 32]
        || *blake3::hash(data).as_bytes() != digest
    {
        return Err("rejected invalid native receiver payload".into());
    }
    #[cfg(windows)]
    check_windows_directory_chain(inbox_root, false)?;
    let root_metadata =
        fs::symlink_metadata(inbox_root).map_err(|error| format!("receiver root: {error}"))?;
    if !root_metadata.file_type().is_dir() {
        return Err("native receiver root is not a real directory".into());
    }
    let stage_name = format!(".akula-native-received-{}.stage", Uuid::new_v4());
    #[cfg(not(windows))]
    let path = inbox_root.join(&stage_name);
    #[cfg(not(windows))]
    let _cleanup = RemoveOnDrop(path.clone());
    // Windows: the opened parent HANDLE and new stage HANDLE remain live
    // through verification; no ambient pathname is used to create this stage.
    #[cfg(windows)]
    let (pinned_root, mut file) = create_pinned_windows_inbox_stage(inbox_root, &stage_name)
        .map_err(|error| format!("Windows pinned Inbox stage refused: {error:?}"))?;
    #[cfg(windows)]
    let mut stage_cleanup = PinnedStageCleanup(Some(file.try_clone().map_err(|error| {
        // A rare handle-duplication failure cannot result in a remote ACK.
        // Best-effort delete the original stage by HANDLE, never by pathname.
        let _ = mark_pinned_windows_stage_delete_on_close(&file);
        format!("duplicate pinned Windows stage cleanup handle: {error}")
    })?));
    // Before opening SQLite or its objects path, refuse insecure existing
    // components. New components may not exist until the first receipt.
    #[cfg(windows)]
    audit_pinned_windows_inbox_storage(inbox_root, &pinned_root, false)
        .map_err(|error| format!("Windows Inbox ACL preflight refused: {error:?}"))?;
    // Bootstrap and pin objects and the SQLite main file relative to the
    // original directory, with no FILE_SHARE_DELETE, for the entire receipt.
    // rusqlite's WAL/SHM and ancestor paths remain outside this lease.
    #[cfg(windows)]
    let sqlite_custody = pin_windows_inbox_sqlite_custody(inbox_root, &pinned_root)
        .map_err(|error| format!("Windows Inbox SQLite custody refused: {error:?}"))?;
    #[cfg(not(windows))]
    let mut file = {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&path)
            .map_err(|error| format!("create private receiver stage: {error}"))?
    };
    file.write_all(data)
        .map_err(|error| format!("write private receiver stage: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync private receiver stage: {error}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("rewind pinned receiver stage: {error}"))?;
    #[cfg(windows)]
    let saved = persist_received_with_publisher(
        &mut file,
        inbox_root,
        size,
        digest,
        sender,
        receiver,
        |stage, _destination, transfer_id| {
            // The ledger's already-authenticated UUID is the only leaf name.
            // The object is written/renamed solely through pinned NT handles.
            publish_pinned_windows_inbox_object(
                inbox_root,
                &pinned_root,
                stage,
                transfer_id,
                size,
                digest,
            )
            .map_err(|_| LedgerError::InvalidLocalObject)
        },
    )?;
    #[cfg(not(windows))]
    let saved = persist_received(&mut file, inbox_root, size, digest, sender, receiver)?;
    // Verify that freshly created SQLite and objects now belong to the same
    // pinned root, with non-writable-by-others ACLs. A failed audit never
    // becomes a verified remote ACK, even if SQLite already wrote local bytes.
    #[cfg(windows)]
    audit_pinned_windows_inbox_storage(inbox_root, &pinned_root, true)
        .map_err(|error| format!("Windows Inbox ACL postflight refused: {error:?}"))?;
    #[cfg(windows)]
    sqlite_custody
        .verify(inbox_root, &pinned_root)
        .map_err(|error| format!("Windows Inbox SQLite custody changed: {error:?}"))?;
    // Even if path-based SQLite reported VerifiedLocal, the exact UUID blob
    // must independently survive an NT handle-relative ACL, hard-link and
    // full-byte BLAKE3 inspection before confirming receipt over the network.
    #[cfg(windows)]
    verify_pinned_windows_inbox_object(inbox_root, &pinned_root, saved.id, size, digest)
        .map_err(|error| format!("Windows Inbox pinned blob verification refused: {error:?}"))?;
    #[cfg(windows)]
    stage_cleanup.finish()?;
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_recipient_persists_only_inbox_and_reuses_exact_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("received-inbox");
        fs::create_dir(&root).unwrap();
        let bytes = b"Native receiver goes to Inbox, not user Vault";
        let digest = *blake3::hash(bytes).as_bytes();
        let source = [77; 32];
        let target = [78; 32];
        let first =
            persist_received_payload(bytes, &root, bytes.len() as u64, digest, source, target)
                .unwrap();
        let replay =
            persist_received_payload(bytes, &root, bytes.len() as u64, digest, source, target)
                .unwrap();
        assert_eq!(first, replay);
        let ledger = InboxLedger::open(&root).unwrap();
        assert_eq!(ledger.count().unwrap(), 1);
        assert_eq!(ledger.status(first.id).unwrap(), LocalStatus::VerifiedLocal);
        assert_eq!(fs::read(ledger.object_path(first.id)).unwrap(), bytes);
        let stray_stages = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".stage"));
        assert!(!stray_stages);
    }

    #[test]
    fn native_receiver_rejects_corrupted_frame_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("received-inbox");
        fs::create_dir(&root).unwrap();
        let bytes = b"no forged recipient file";
        assert!(
            persist_received_payload(
                bytes,
                &root,
                bytes.len() as u64,
                *blake3::hash(b"forgery").as_bytes(),
                [7; 32],
                [8; 32],
            )
            .is_err()
        );
        assert!(!root.join("objects").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_receiver_refuses_junction_ancestor_before_creating_inbox() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        let junction = dir.path().join("junction");
        fs::create_dir(&real).unwrap();
        let mklink = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&real)
            .output()
            .expect("Windows cmd is required for junction regression");
        assert!(
            mklink.status.success(),
            "mklink /J failed: {}",
            String::from_utf8_lossy(&mklink.stderr)
        );
        let target = junction.join("Akula").join("received-inbox");
        assert!(check_windows_directory_chain(&target, true).is_err());
        assert!(!real.join("Akula").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_receiver_requires_existing_regular_directory_chain() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join("received-inbox");
        assert!(check_windows_directory_chain(&inbox, false).is_err());
        fs::create_dir(&inbox).unwrap();
        check_windows_directory_chain(&inbox, false).unwrap();
        let file = inbox.join("not-a-directory");
        fs::write(&file, b"not a directory").unwrap();
        assert!(check_windows_directory_chain(&file, false).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_receiver_rejects_inbox_with_everyone_full_control() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("received-inbox");
        fs::create_dir(&root).unwrap();
        let grant = std::process::Command::new("icacls")
            .arg(&root)
            .args(["/grant", "*S-1-1-0:(OI)(CI)F"])
            .output()
            .expect("icacls is required for Windows DACL regression");
        assert!(
            grant.status.success(),
            "icacls grant failed: {}",
            String::from_utf8_lossy(&grant.stderr)
        );
        assert!(create_pinned_windows_inbox_stage(&root, "untrusted.stage").is_err());
        assert!(!root.join("untrusted.stage").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_sqlite_custody_blocks_rename_and_preserves_parent_identity() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("received-inbox");
        fs::create_dir(&root).unwrap();
        let (pinned_root, stage) =
            create_pinned_windows_inbox_stage(&root, "sqlite-lease.stage").unwrap();
        let lease = pin_windows_inbox_sqlite_custody(&root, &pinned_root).unwrap();
        let objects = root.join("objects");
        let database = root.join("received-inbox.sqlite3");
        assert!(database.is_file());
        assert!(objects.is_dir());

        // No FILE_SHARE_DELETE on the opened real file/directory handles.
        // A cooperating process cannot replace either name while rusqlite
        // uses the path-based SQLite connection.
        assert!(fs::rename(&database, root.join("moved.sqlite3")).is_err());
        assert!(fs::rename(&objects, root.join("moved-objects")).is_err());
        assert!(fs::rename(&root, dir.path().join("moved-inbox")).is_err());
        lease.verify(&root, &pinned_root).unwrap();
        drop(lease);
        mark_pinned_windows_stage_delete_on_close(&stage).unwrap();
        drop(stage);
        drop(pinned_root);
        assert!(database.is_file());
        assert!(objects.is_dir());
    }

    #[cfg(windows)]
    #[test]
    fn windows_pinned_objects_writer_is_no_replace_and_removes_failed_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("received-inbox");
        fs::create_dir(&root).unwrap();
        // The legacy ledger creates its trusted, private schema and objects
        // directory; the P2P writer itself never opens a global object path.
        drop(InboxLedger::open(&root).unwrap());
        let (pinned_root, mut stage) =
            create_pinned_windows_inbox_stage(&root, "fixed-stage.bin").unwrap();
        let bytes = b"one verified pinned NT object";
        let digest = *blake3::hash(bytes).as_bytes();
        let id = Uuid::new_v4();
        stage.write_all(bytes).unwrap();
        stage.sync_all().unwrap();
        stage.seek(SeekFrom::Start(0)).unwrap();

        publish_pinned_windows_inbox_object(
            &root,
            &pinned_root,
            &mut stage,
            id,
            bytes.len() as u64,
            digest,
        )
        .unwrap();
        let output = root
            .join("objects")
            .join(format!("{}.blob", id.hyphenated()));
        assert_eq!(fs::read(&output).unwrap(), bytes);

        // Even the same authenticated UUID is not permitted to overwrite an
        // already published leaf. The pre-existing bytes must remain intact.
        assert!(
            publish_pinned_windows_inbox_object(
                &root,
                &pinned_root,
                &mut stage,
                id,
                bytes.len() as u64,
                digest,
            )
            .is_err()
        );
        assert_eq!(fs::read(&output).unwrap(), bytes);
        let partials = fs::read_dir(root.join("objects"))
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".part"));
        assert!(
            !partials,
            "failed NT no-replace must clean its temporary object"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_pinned_stage_cleanup_never_deletes_replaced_name() {
        for explicit_success in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("received-inbox");
            fs::create_dir(&root).unwrap();
            let (pinned_root, mut stage) =
                create_pinned_windows_inbox_stage(&root, "pending.stage").unwrap();
            stage.write_all(b"actual original stage").unwrap();
            stage.sync_all().unwrap();
            let mut cleanup = PinnedStageCleanup(Some(stage.try_clone().unwrap()));
            let original_name = root.join("pending.stage");
            let renamed_original = root.join("moved-original.stage");
            // Simulate a same-user replacement of the former pathname while
            // the originally created DELETE_ACCESS stage HANDLE remains open.
            fs::rename(&original_name, &renamed_original).unwrap();
            fs::write(&original_name, b"unrelated replacement").unwrap();
            if explicit_success {
                cleanup.finish().unwrap();
            }
            drop(cleanup);
            drop(stage);
            drop(pinned_root);
            assert_eq!(fs::read(&original_name).unwrap(), b"unrelated replacement");
            assert!(
                !renamed_original.exists(),
                "only the opened stage may be removed"
            );
        }
    }

    #[cfg(windows)]
    fn grant_everyone_full_control(path: &Path, directory: bool) {
        // A file has no children: (OI)(CI) is a directory inheritance
        // directive, not reliable evidence of an effective file write ACE.
        // The SQLite regression must grant *this file* explicit Full Control.
        let ace = if directory {
            "*S-1-1-0:(OI)(CI)F"
        } else {
            "*S-1-1-0:F"
        };
        let grant = std::process::Command::new("icacls")
            .arg(path)
            .args(["/grant", ace])
            .output()
            .expect("icacls is required for Windows P2P Inbox ACL regression");
        assert!(
            grant.status.success(),
            "icacls refused: {}",
            String::from_utf8_lossy(&grant.stderr)
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_receiver_refuses_insecure_objects_acl_before_second_network_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("received-inbox");
        fs::create_dir(&root).unwrap();
        let original = b"verified first receipt";
        persist_received_payload(
            original,
            &root,
            original.len() as u64,
            *blake3::hash(original).as_bytes(),
            [41; 32],
            [51; 32],
        )
        .unwrap();
        grant_everyone_full_control(&root.join("objects"), true);
        let other = b"second file must not enter exposed objects";
        assert!(
            persist_received_payload(
                other,
                &root,
                other.len() as u64,
                *blake3::hash(other).as_bytes(),
                [42; 32],
                [51; 32],
            )
            .is_err()
        );
        let cold = InboxLedger::open(&root).unwrap();
        assert_eq!(cold.count().unwrap(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn windows_receiver_refuses_insecure_sqlite_acl_before_second_network_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("received-inbox");
        fs::create_dir(&root).unwrap();
        let original = b"verified first sqlite receipt";
        persist_received_payload(
            original,
            &root,
            original.len() as u64,
            *blake3::hash(original).as_bytes(),
            [61; 32],
            [71; 32],
        )
        .unwrap();
        grant_everyone_full_control(&root.join("received-inbox.sqlite3"), false);
        let other = b"second receipt must not enter exposed sqlite";
        assert!(
            persist_received_payload(
                other,
                &root,
                other.len() as u64,
                *blake3::hash(other).as_bytes(),
                [62; 32],
                [71; 32],
            )
            .is_err()
        );
        let cold = InboxLedger::open(&root).unwrap();
        assert_eq!(cold.count().unwrap(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn windows_receiver_refuses_insecure_blob_acl_on_duplicate_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("received-inbox");
        fs::create_dir(&root).unwrap();
        let bytes = b"verified object is not allowed to become public";
        let digest = *blake3::hash(bytes).as_bytes();
        let initial =
            persist_received_payload(bytes, &root, bytes.len() as u64, digest, [81; 32], [91; 32])
                .unwrap();
        let ledger = InboxLedger::open(&root).unwrap();
        let object = ledger.object_path(initial.id);
        drop(ledger);
        grant_everyone_full_control(&object, false);
        // Path-based SQLite can still find the correct bytes, but the
        // root-relative final object ACL check must fail before a new ACK.
        assert!(
            persist_received_payload(bytes, &root, bytes.len() as u64, digest, [81; 32], [91; 32],)
                .is_err()
        );
        assert_eq!(InboxLedger::open(&root).unwrap().count().unwrap(), 1);
        assert_eq!(fs::read(object).unwrap(), bytes);
    }

    #[cfg(windows)]
    #[test]
    fn windows_receiver_refuses_hardlinked_blob_on_duplicate_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("received-inbox");
        fs::create_dir(&root).unwrap();
        let bytes = b"verified object is not allowed to have an outside alias";
        let digest = *blake3::hash(bytes).as_bytes();
        let initial =
            persist_received_payload(bytes, &root, bytes.len() as u64, digest, [82; 32], [92; 32])
                .unwrap();
        let ledger = InboxLedger::open(&root).unwrap();
        let object = ledger.object_path(initial.id);
        drop(ledger);
        let other_link = root.join("alias-outside-objects.bin");
        fs::hard_link(&object, &other_link).unwrap();
        assert!(
            persist_received_payload(bytes, &root, bytes.len() as u64, digest, [82; 32], [92; 32],)
                .is_err()
        );
        assert_eq!(fs::read(&other_link).unwrap(), bytes);
        assert_eq!(InboxLedger::open(&root).unwrap().count().unwrap(), 1);
    }

    fn stage(root: &Path, data: &[u8]) -> File {
        let path = root.join("verified-stage.bin");
        fs::write(&path, data).unwrap();
        File::open(path).unwrap()
    }

    #[test]
    fn cold_received_inbox_replays_only_one_verified_object() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"one first-beta authenticated local file";
        let mut file = stage(dir.path(), bytes);
        let digest = *blake3::hash(bytes).as_bytes();
        let root = dir.path().join("inbox");
        let sender = [11u8; 32];
        let receiver = [12u8; 32];
        let first = persist_received(
            &mut file,
            &root,
            bytes.len() as u64,
            digest,
            sender,
            receiver,
        )
        .unwrap();
        let replay = persist_received(
            &mut file,
            &root,
            bytes.len() as u64,
            digest,
            sender,
            receiver,
        )
        .unwrap();
        assert_eq!(first, replay);
        let cold = InboxLedger::open(&root).unwrap();
        assert_eq!(cold.count().unwrap(), 1);
        assert_eq!(cold.status(first.id).unwrap(), LocalStatus::VerifiedLocal);
        assert_eq!(fs::read(cold.object_path(first.id)).unwrap(), bytes);
    }

    #[test]
    fn altered_local_received_object_fails_closed_on_retry() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"valid object before tampering";
        let mut file = stage(dir.path(), bytes);
        let digest = *blake3::hash(bytes).as_bytes();
        let root = dir.path().join("inbox");
        let sender = [17u8; 32];
        let receiver = [18u8; 32];
        let intent = persist_received(
            &mut file,
            &root,
            bytes.len() as u64,
            digest,
            sender,
            receiver,
        )
        .unwrap();
        let ledger = InboxLedger::open(&root).unwrap();
        let path = ledger.object_path(intent.id);
        fs::write(&path, b"tampered").unwrap();
        drop(ledger);
        assert!(
            persist_received(
                &mut file,
                &root,
                bytes.len() as u64,
                digest,
                sender,
                receiver,
            )
            .is_err()
        );
        let cold = InboxLedger::open(&root).unwrap();
        assert_eq!(cold.status(intent.id).unwrap(), LocalStatus::NeedsReview);
        assert_eq!(cold.count().unwrap(), 1);
        assert_eq!(fs::read(path).unwrap(), b"tampered");
    }

    #[test]
    fn different_sender_does_not_collide_on_identical_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"duplicate bytes from different devices";
        let mut file = stage(dir.path(), bytes);
        let digest = *blake3::hash(bytes).as_bytes();
        let root = dir.path().join("inbox");
        let first = persist_received(
            &mut file,
            &root,
            bytes.len() as u64,
            digest,
            [1; 32],
            [2; 32],
        )
        .unwrap();
        let second = persist_received(
            &mut file,
            &root,
            bytes.len() as u64,
            digest,
            [3; 32],
            [2; 32],
        )
        .unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(InboxLedger::open(&root).unwrap().count().unwrap(), 2);
    }
}
