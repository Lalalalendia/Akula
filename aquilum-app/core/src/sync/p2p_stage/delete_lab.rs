//! Windows 7 same-HANDLE delete probe. QA-only; no shipping API or GUI.
use super::{pinned_windows, VaultPortError};
use std::{
    ffi::c_void,
    fs,
    io::Read,
    os::windows::{fs::MetadataExt, io::AsRawHandle},
    path::Path,
};

mod relative;
use akula_inbox_ledger_lab::{InboxLedger, RemovalDispositionWitness};
use relative::open_delete_candidate;
use uuid::Uuid;

const REPARSE_ATTRIBUTE: u32 = 0x0000_0400;
const FILE_DISPOSITION_INFO: u32 = 4;
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// The directories and SQLite pathname must remain the same OS objects
/// from consent through the root-relative disposition. This is only an
/// in-process Windows fixture guard, not an OS-authenticated SQLite connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PinnedInboxIdentity {
    root: (u32, u64),
    objects: (u32, u64),
    database: (u32, u64),
}

#[link(name = "kernel32")]
extern "system" {
    fn SetFileInformationByHandle(
        handle: *mut c_void,
        class: u32,
        info: *mut c_void,
        size: u32,
    ) -> i32;
}

/// Lab-only. Production still requires explicit authenticated SQLite consent.
fn delete_disposable_fixture(
    root: &Path,
    id: Uuid,
    expected_bytes: u64,
    expected_hash: [u8; 32],
) -> Result<(), VaultPortError> {
    delete_disposable_fixture_matching(root, id, expected_bytes, expected_hash, None, None)
        .map(|_| ())
}

/// Require the OS identity of the independently verified Ledger file as well
/// as equality between the root-relative witness and DELETE-capable handle.
fn delete_disposable_fixture_matching(
    root: &Path,
    id: Uuid,
    expected_bytes: u64,
    expected_hash: [u8; 32],
    ledger_identity: Option<(u32, u64, u32)>,
    expected_custody: Option<PinnedInboxIdentity>,
) -> Result<(u32, u64, u32), VaultPortError> {
    if id.is_nil() || expected_bytes == 0 || expected_bytes > MAX_BYTES || expected_hash == [0; 32]
    {
        return Err(VaultPortError::UnsafePath);
    }
    let (root_guard, objects, database_guard) = super::pin_p2p_read_snapshot(root)?;
    if let Some(expected) = expected_custody {
        let now = PinnedInboxIdentity {
            root: pinned_windows::file_identity(&root_guard)?,
            objects: pinned_windows::file_identity(&objects)?,
            database: pinned_windows::file_identity(&database_guard)?,
        };
        if now != expected {
            return Err(VaultPortError::FileChanged);
        }
    }
    let name = format!("{}.blob", id.hyphenated());
    // Stable directory-relative witness; no sender-controlled path.
    let witness = pinned_windows::open_child(&objects, &name, true)?;
    let identity = pinned_windows::file_identity_with_links(&witness)?;
    if identity.2 != 1 || ledger_identity.is_some_and(|original| original != identity) {
        return Err(VaultPortError::FileChanged);
    }

    // Open DELETE-capable HANDLE using NtCreateFile RootDirectory = the
    // pinned objects directory. Never re-resolve its ambient pathname.
    let mut candidate = open_delete_candidate(&objects, &name)?;
    let meta = candidate
        .metadata()
        .map_err(|e| VaultPortError::Io(e.to_string()))?;
    if !meta.is_file()
        || meta.file_attributes() & REPARSE_ATTRIBUTE != 0
        || meta.len() != expected_bytes
        || pinned_windows::file_identity_with_links(&candidate)? != identity
    {
        return Err(VaultPortError::FileChanged);
    }
    let mut total = 0u64;
    let mut hasher = blake3::Hasher::new();
    let mut block = [0u8; 64 * 1024];
    loop {
        let n = candidate
            .read(&mut block)
            .map_err(|e| VaultPortError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .ok_or(VaultPortError::FileChanged)?;
        if total > expected_bytes {
            return Err(VaultPortError::FileChanged);
        }
        hasher.update(&block[..n]);
    }
    if total != expected_bytes
        || *hasher.finalize().as_bytes() != expected_hash
        || candidate
            .metadata()
            .map_err(|e| VaultPortError::Io(e.to_string()))?
            .len()
            != expected_bytes
        || pinned_windows::file_identity_with_links(&candidate)? != identity
    {
        return Err(VaultPortError::FileChanged);
    }
    drop(witness);
    // Win7-compatible deletion is marked on the checked HANDLE, not a path.
    let mut disposition = 1u8;
    // SAFETY: candidate holds DELETE access; synchronous API receives one
    // valid FILE_DISPOSITION_INFO byte and a live OS file handle.
    let ok = unsafe {
        SetFileInformationByHandle(
            candidate.as_raw_handle(),
            FILE_DISPOSITION_INFO,
            (&mut disposition as *mut u8).cast::<c_void>(),
            1,
        )
    };
    if ok == 0 {
        return Err(VaultPortError::Io(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    drop(candidate);
    // Verify absence using the pinned objects HANDLE. Delete-pending and any
    // unexpected replacement remain errors, never a successful receipt.
    if pinned_windows::open_optional_child(&objects, &name)?.is_some() {
        return Err(VaultPortError::FileChanged);
    }
    // Return identity of the verified DELETE-capable HANDLE only after the
    // kernel disposition and root-relative absence check both succeed.
    Ok(identity)
}

/// Synthetic explicit user confirmation; no GUI can invoke this test module.
/// This is never derived from sender-provided paths or network metadata.
#[derive(Clone, Copy)]
struct ConfirmedRemoval {
    transfer_id: Uuid,
    bytes: u64,
    blake3: [u8; 32],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CrashCut {
    None,
    AfterRequest,
    AfterDisposition,
}

/// Test-only consent -> Ledger CAS -> same Windows HANDLE -> Ledger finish.
/// The hook is a synthetic hostile-writer interruption after SQLite commit.
fn delete_after_explicit_consent(
    ledger: &InboxLedger,
    root: &Path,
    selected_id: Uuid,
    consent: Option<ConfirmedRemoval>,
    cut: CrashCut,
    after_request: impl FnOnce(),
) -> Result<(), String> {
    let confirmed = consent.ok_or("explicit confirmation required")?;
    if selected_id.is_nil() || confirmed.transfer_id != selected_id {
        return Err("confirmation does not match selected transfer".into());
    }
    let name = format!("{}.blob", selected_id.hyphenated());
    if ledger.object_path(selected_id) != root.join("objects").join(&name) {
        return Err("ledger does not own selected Inbox root".into());
    }
    // Capture the Inbox root, objects directory and database pathname before
    // committing intent. Keep all three handles live until disposition returns;
    // their OS identities must still match when DELETE is attempted.
    let (root_guard, objects_guard, database_guard) =
        super::pin_p2p_read_snapshot(root)
            .map_err(|e| format!("pinned Inbox custody failed: {e:?}"))?;
    let custody = PinnedInboxIdentity {
        root: pinned_windows::file_identity(&root_guard)
            .map_err(|e| format!("root identity failed: {e:?}"))?,
        objects: pinned_windows::file_identity(&objects_guard)
            .map_err(|e| format!("objects identity failed: {e:?}"))?,
        database: pinned_windows::file_identity(&database_guard)
            .map_err(|e| format!("database identity failed: {e:?}"))?,
    };
    // Capture the verified opened object and its OS identity before consuming
    // the one-shot intent. Size/hash alone cannot detect an identical swap.
    let (file, receipt) = ledger
        .open_verified_file(selected_id)
        .map_err(|e| format!("receipt verification failed: {e:?}"))?;
    if receipt.bytes != confirmed.bytes || receipt.blake3 != confirmed.blake3 {
        return Err("stale or mismatched user confirmation".into());
    }
    let original_identity = pinned_windows::file_identity_with_links(&file)
        .map_err(|e| format!("receipt identity failed: {e:?}"))?;
    if original_identity.2 != 1 {
        return Err("multiply linked receipt rejected".into());
    }
    drop(file);

    ledger
        .request_removal(selected_id)
        .map_err(|e| format!("one-shot SQLite request refused: {e:?}"))?;
    if cut == CrashCut::AfterRequest {
        return Err("simulated crash after durable request".into());
    }
    after_request();

    let observed_identity = delete_disposable_fixture_matching(
        root,
        selected_id,
        receipt.bytes,
        receipt.blake3,
        Some(original_identity),
        Some(custody),
    )
    .map_err(|e| format!("same-HANDLE disposition refused: {e:?}"))?;
    if cut == CrashCut::AfterDisposition {
        return Err("simulated crash after HANDLE disposition".into());
    }
    let witnessed = RemovalDispositionWitness::for_disposable_lab(
        selected_id,
        receipt.bytes,
        receipt.blake3,
        observed_identity.0,
        observed_identity.1,
        observed_identity.2,
    )
    .map_err(|e| format!("invalid HANDLE outcome evidence: {e:?}"))?;
    ledger
        .finish_removal(selected_id, witnessed)
        .map_err(|e| format!("SQLite finish refused: {e:?}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use akula_inbox_ledger_lab::{ExpectedTransfer, RemovalAuditStatus, Transport};

    fn ledger_fixture(
        bytes: &[u8],
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        InboxLedger,
        ExpectedTransfer,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("inbox");
        let mut ledger = InboxLedger::open(&root).unwrap();
        let receipt = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [41; 32],
            recipient_key: [42; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes).as_bytes(),
        };
        ledger.reserve(&receipt).unwrap();
        fs::write(ledger.object_path(receipt.id), bytes).unwrap();
        ledger.confirm(&receipt).unwrap();
        (temp, root, ledger, receipt)
    }

    fn confirmation(receipt: &ExpectedTransfer) -> ConfirmedRemoval {
        ConfirmedRemoval {
            transfer_id: receipt.id,
            bytes: receipt.bytes,
            blake3: receipt.blake3,
        }
    }

    #[test]
    fn confirmed_intent_precedes_verified_same_handle_delete() {
        let bytes = b"consent bound disposable object";
        let (_temp, root, ledger, receipt) = ledger_fixture(bytes);
        let id = receipt.id;
        let consent = confirmation(&receipt);
        assert!(
            delete_after_explicit_consent(&ledger, &root, id, None, CrashCut::None, || {},)
                .is_err()
        );
        assert!(delete_after_explicit_consent(
            &ledger,
            &root,
            id,
            Some(ConfirmedRemoval {
                transfer_id: Uuid::new_v4(),
                ..consent
            }),
            CrashCut::None,
            || {},
        )
        .is_err());
        assert!(delete_after_explicit_consent(
            &ledger,
            &root,
            id,
            Some(ConfirmedRemoval {
                blake3: [91; 32],
                ..consent
            }),
            CrashCut::None,
            || {},
        )
        .is_err());
        assert_eq!(
            ledger.removal_status(id).unwrap(),
            RemovalAuditStatus::Available
        );
        assert_eq!(fs::read(ledger.object_path(id)).unwrap(), bytes);

        delete_after_explicit_consent(&ledger, &root, id, Some(consent), CrashCut::None, || {
            assert_eq!(
                ledger.removal_status(id).unwrap(),
                RemovalAuditStatus::Requested
            );
            assert!(ledger.object_path(id).exists());
        })
        .unwrap();
        assert_eq!(
            ledger.removal_status(id).unwrap(),
            RemovalAuditStatus::Removed
        );
        assert!(!ledger.object_path(id).exists());
        assert!(delete_after_explicit_consent(
            &ledger,
            &root,
            id,
            Some(consent),
            CrashCut::None,
            || {},
        )
        .is_err());
        drop(ledger);
        let cold = InboxLedger::open_existing(&root).unwrap().unwrap();
        assert_eq!(cold.removal_status(id).unwrap(), RemovalAuditStatus::Removed);
    }

    #[test]
    fn crash_after_sqlite_request_retains_original_and_one_shot_intent() {
        let bytes = b"crash before HANDLE disposition";
        let (_temp, root, ledger, receipt) = ledger_fixture(bytes);
        let id = receipt.id;
        assert!(delete_after_explicit_consent(
            &ledger,
            &root,
            id,
            Some(confirmation(&receipt)),
            CrashCut::AfterRequest,
            || {},
        )
        .is_err());
        drop(ledger);
        let reader = InboxLedger::open_existing(&root).unwrap().unwrap();
        assert_eq!(
            reader.removal_status(id).unwrap(),
            RemovalAuditStatus::Requested
        );
        assert_eq!(fs::read(reader.object_path(id)).unwrap(), bytes);
        assert!(reader.open_verified_file(id).is_err());
    }

    #[test]
    fn crash_after_disposition_before_sqlite_finish_requires_review() {
        let bytes = b"crash after HANDLE disposition";
        let (_temp, root, ledger, receipt) = ledger_fixture(bytes);
        let id = receipt.id;
        assert!(delete_after_explicit_consent(
            &ledger,
            &root,
            id,
            Some(confirmation(&receipt)),
            CrashCut::AfterDisposition,
            || {},
        )
        .is_err());
        drop(ledger);
        let reader = InboxLedger::open_existing(&root).unwrap().unwrap();
        assert!(!reader.object_path(id).exists());
        assert_eq!(
            reader.removal_status(id).unwrap(),
            RemovalAuditStatus::NeedsReview
        );
        assert!(reader.open_verified_file(id).is_err());
    }

    #[test]
    fn identical_bytes_replacement_cannot_reuse_original_file_identity() {
        let bytes = b"same bytes but another Windows file index";
        let (_temp, root, ledger, receipt) = ledger_fixture(bytes);
        let id = receipt.id;
        let leaf = ledger.object_path(id);
        let preserved = root.join("objects").join("preserved-original");
        assert!(delete_after_explicit_consent(
            &ledger,
            &root,
            id,
            Some(confirmation(&receipt)),
            CrashCut::None,
            || {
                fs::rename(&leaf, &preserved).unwrap();
                fs::write(&leaf, bytes).unwrap();
            },
        )
        .is_err());
        assert_eq!(fs::read(&leaf).unwrap(), bytes);
        assert_eq!(fs::read(&preserved).unwrap(), bytes);
        assert_eq!(
            ledger.removal_status(id).unwrap(),
            RemovalAuditStatus::Requested
        );
    }

    #[test]
    fn swapped_objects_directory_with_identical_bytes_is_not_deleted() {
        let bytes = b"directory swap with identical verified bytes";
        let (_temp, root, ledger, receipt) = ledger_fixture(bytes);
        let id = receipt.id;
        let old_objects = root.join("objects");
        let preserved = root.join("original-objects");
        let injected = root.join("replacement-objects");
        fs::create_dir(&injected).unwrap();
        let name = format!("{}.blob", id.hyphenated());
        fs::write(injected.join(&name), bytes).unwrap();

        let result = delete_after_explicit_consent(
            &ledger,
            &root,
            id,
            Some(confirmation(&receipt)),
            CrashCut::None,
            || {
                fs::rename(&old_objects, &preserved).unwrap();
                fs::rename(&injected, &old_objects).unwrap();
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(preserved.join(&name)).unwrap(), bytes);
        assert_eq!(fs::read(old_objects.join(&name)).unwrap(), bytes);
        assert_eq!(
            ledger.removal_status(id).unwrap(),
            RemovalAuditStatus::Requested
        );
    }

    #[test]
    fn exclusive_open_after_request_blocks_windows_delete_and_cold_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        let bytes = b"exclusive sharing violation after SQLite CAS";
        let (_temp, root, ledger, receipt) = ledger_fixture(bytes);
        let id = receipt.id;
        let leaf = ledger.object_path(id);
        let mut exclusive = None;

        let result = delete_after_explicit_consent(
            &ledger,
            &root,
            id,
            Some(confirmation(&receipt)),
            CrashCut::None,
            || {
                exclusive = Some(
                    fs::OpenOptions::new()
                        .read(true)
                        .share_mode(0)
                        .open(&leaf)
                        .unwrap(),
                );
            },
        );
        assert!(result.is_err());
        drop(exclusive);
        assert_eq!(fs::read(&leaf).unwrap(), bytes);
        drop(ledger);
        let cold = InboxLedger::open_existing(&root).unwrap().unwrap();
        assert_eq!(
            cold.removal_status(id).unwrap(),
            RemovalAuditStatus::Requested
        );
        assert!(cold.open_verified_file(id).is_err());
        assert_eq!(fs::read(cold.object_path(id)).unwrap(), bytes);
    }

    #[test]
    fn wrong_inbox_root_rejects_confirmation_before_sqlite_request() {
        let bytes = b"wrong authorized root";
        let (_temp, root, ledger, receipt) = ledger_fixture(bytes);
        let wrong_root = root.join("other-inbox");
        assert!(delete_after_explicit_consent(
            &ledger,
            &wrong_root,
            receipt.id,
            Some(confirmation(&receipt)),
            CrashCut::None,
            || {},
        )
        .is_err());
        assert_eq!(
            ledger.removal_status(receipt.id).unwrap(),
            RemovalAuditStatus::Available
        );
        assert_eq!(fs::read(ledger.object_path(receipt.id)).unwrap(), bytes);
    }

    #[test]
    fn competing_confirmed_sqlite_connections_allow_one_disposition() {
        use std::sync::{Arc, Barrier};
        let bytes = b"only one competing physical disposition";
        let (_temp, root, ledger, receipt) = ledger_fixture(bytes);
        drop(ledger);
        let first = InboxLedger::open(&root).unwrap();
        let second = InboxLedger::open(&root).unwrap();
        let gate = Arc::new(Barrier::new(2));
        let consent = confirmation(&receipt);
        let id = receipt.id;
        let root1 = root.clone();
        let root2 = root.clone();
        let g1 = Arc::clone(&gate);
        let g2 = Arc::clone(&gate);
        let t1 = std::thread::spawn(move || {
            g1.wait();
            delete_after_explicit_consent(&first, &root1, id, Some(consent), CrashCut::None, || {})
                .is_ok()
        });
        let t2 = std::thread::spawn(move || {
            g2.wait();
            delete_after_explicit_consent(&second, &root2, id, Some(consent), CrashCut::None, || {})
                .is_ok()
        });
        assert_ne!(t1.join().unwrap(), t2.join().unwrap());
        let reader = InboxLedger::open_existing(&root).unwrap().unwrap();
        assert_eq!(
            reader.removal_status(id).unwrap(),
            RemovalAuditStatus::Removed
        );
        assert!(!reader.object_path(id).exists());
    }

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("inbox");
        fs::create_dir_all(root.join("objects")).unwrap();
        fs::write(root.join("received-inbox.sqlite3"), b"dummy").unwrap();
        (dir, root)
    }

    #[test]
    fn only_verified_uuid_leaf_is_deleted() {
        let (_temp, root) = fixture();
        let id = Uuid::new_v4();
        let bytes = b"checked fixture";
        let leaf = root
            .join("objects")
            .join(format!("{}.blob", id.hyphenated()));
        let other = root.join("objects").join("unrelated");
        fs::write(&leaf, bytes).unwrap();
        fs::write(&other, b"retain").unwrap();
        assert!(delete_disposable_fixture(
            &root,
            Uuid::new_v4(),
            bytes.len() as u64,
            *blake3::hash(bytes).as_bytes(),
        )
        .is_err());
        assert_eq!(fs::read(&leaf).unwrap(), bytes);
        delete_disposable_fixture(
            &root,
            id,
            bytes.len() as u64,
            *blake3::hash(bytes).as_bytes(),
        )
        .unwrap();
        assert!(!leaf.exists());
        assert_eq!(fs::read(&other).unwrap(), b"retain");
    }

    #[test]
    fn refuses_bad_hash_size_and_multiple_hard_links() {
        let (_temp, root) = fixture();
        let id = Uuid::new_v4();
        let bytes = b"still owned";
        let leaf = root
            .join("objects")
            .join(format!("{}.blob", id.hyphenated()));
        fs::write(&leaf, bytes).unwrap();
        assert!(delete_disposable_fixture(&root, id, bytes.len() as u64, [9; 32]).is_err());
        assert!(delete_disposable_fixture(
            &root,
            id,
            bytes.len() as u64 + 1,
            *blake3::hash(bytes).as_bytes(),
        )
        .is_err());
        let alias = root.join("objects").join("hard-link");
        fs::hard_link(&leaf, &alias).unwrap();
        assert!(delete_disposable_fixture(
            &root,
            id,
            bytes.len() as u64,
            *blake3::hash(bytes).as_bytes(),
        )
        .is_err());
        assert_eq!(fs::read(&leaf).unwrap(), bytes);
        assert_eq!(fs::read(&alias).unwrap(), bytes);
    }
}
