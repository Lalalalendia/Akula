//! Fail-closed P2P Inbox to registered Vault create-new File Gate adapter.
//! Experimental QA only: the signed installed app must not enable P2P yet.
use crate::{app_core::Core, files::error::FileCommandError};
use std::{
    fs::File,
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// Only locally reverified Inbox handles and commitments may enter this API.
/// Never use a sender-provided name; the immutable Inbox original remains in
/// place on success or failure. This is not a durable remote delivery ACK.
pub fn import_verified_inbox_as_new(
    core: &Core,
    inbox_root: &Path,
    vault_root: &Path,
    stage: &mut File,
    expected_bytes: u64,
    expected_blake3: [u8; 32],
    receipt: Uuid,
) -> Result<PathBuf, FileCommandError> {
    #[cfg(not(windows))]
    {
        let _ = (
            core,
            inbox_root,
            vault_root,
            stage,
            expected_bytes,
            expected_blake3,
            receipt,
        );
        Err(FileCommandError::Task {
            message: "Inbox to Vault import is Windows QA-only and release HOLD".into(),
        })
    }
    #[cfg(windows)]
    {
        import_windows(
            core,
            inbox_root,
            vault_root,
            stage,
            expected_bytes,
            expected_blake3,
            receipt,
        )
    }
}

#[cfg(windows)]
fn import_windows(
    core: &Core,
    inbox_root: &Path,
    vault_root: &Path,
    stage: &mut File,
    expected_bytes: u64,
    expected_blake3: [u8; 32],
    receipt: Uuid,
) -> Result<PathBuf, FileCommandError> {
    use super::{pin_windows_export_parent, pin_windows_inbox_read, save_verified_stage_as_new};
    use std::{fs, io::Read};

    fn refused(message: impl Into<String>) -> FileCommandError {
        FileCommandError::Task {
            message: message.into(),
        }
    }
    if receipt.is_nil()
        || !inbox_root.is_absolute()
        || !vault_root.is_absolute()
        || expected_bytes == 0
        || expected_bytes > super::MAX_INBOX_ITEM_BYTES
        || expected_blake3 == [0; 32]
    {
        return Err(refused("Invalid Inbox receipt or root"));
    }
    // The handles pin the complete NT ancestor chains against rename.
    let _vault_pin = pin_windows_export_parent(vault_root)
        .map_err(|e| refused(format!("Vault directory custody refused: {e:?}")))?;
    let _source_pin = pin_windows_inbox_read(inbox_root)
        .map_err(|e| refused(format!("Inbox directory custody refused: {e:?}")))?;
    let vault = fs::canonicalize(vault_root)?;
    let inbox = fs::canonicalize(inbox_root)?;
    if vault.starts_with(&inbox) || inbox.starts_with(&vault) {
        return Err(refused("Inbox and Vault roots overlap"));
    }
    let registered = core
        .ui_state
        .workspace_roots()
        .map_err(|e| refused(format!("Registered Vault lookup failed: {e:?}")))?;
    if !registered
        .into_iter()
        .any(|root| fs::canonicalize(root).is_ok_and(|canon| canon == vault))
    {
        return Err(refused("Selected Vault is not registered in this Core"));
    }

    let destination = vault.join(format!("received-{receipt}.bin"));
    #[cfg(feature = "host-custody")]
    let _epoch = core.ordinary_note_publication_lease()?;
    {
        // Same shared File Gate interlock and S3/HOST custody HOLD as create.
        let _guard = core.file_mutation_guard_for_write()?;
        let published =
            save_verified_stage_as_new(stage, expected_bytes, expected_blake3, &destination)
                .map_err(|e| match e {
                    super::InboxError::AlreadyExists => FileCommandError::AlreadyExists {
                        path: destination.to_string_lossy().into_owned(),
                    },
                    _ => refused(format!("Verified Inbox publication refused: {e:?}")),
                })?;
        if published.bytes != expected_bytes || published.blake3_root != expected_blake3 {
            return Err(FileCommandError::NeedsReview {
                message: "File Gate receipt mismatch, keep Inbox source".into(),
            });
        }
        // Cold verification after atomic create-NO-REPLACE. Never automatically
        // unlink an ambiguous published file: keep the Inbox copy for review.
        let mut cold = File::open(&destination).map_err(|e| FileCommandError::NeedsReview {
            message: format!("Cannot cold-open new Vault file: {e}"),
        })?;
        let mut hasher = blake3::Hasher::new();
        let mut size = 0u64;
        let mut buf = [0u8; 65536];
        loop {
            let n = cold
                .read(&mut buf)
                .map_err(|e| FileCommandError::NeedsReview {
                    message: format!("Cold-read failed after File Gate create: {e}"),
                })?;
            if n == 0 {
                break;
            }
            size = size
                .checked_add(n as u64)
                .ok_or_else(|| FileCommandError::NeedsReview {
                    message: "Cold-verified byte count overflow".into(),
                })?;
            if size > expected_bytes {
                return Err(FileCommandError::NeedsReview {
                    message: "Vault bytes changed after publication".into(),
                });
            }
            hasher.update(&buf[..n]);
        }
        if !cold.metadata()?.is_file()
            || cold.metadata()?.len() != expected_bytes
            || size != expected_bytes
            || *hasher.finalize().as_bytes() != expected_blake3
        {
            return Err(FileCommandError::NeedsReview {
                message: "Vault cold check failed, original Inbox retained".into(),
            });
        }
    }
    core.search.notify_paths(vec![destination.clone()]);
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    fn fixture() -> (
        tempfile::TempDir,
        std::sync::Arc<Core>,
        PathBuf,
        PathBuf,
        PathBuf,
    ) {
        use crate::app_core::CoreEvent;
        use std::sync::Arc;
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("app");
        let vault = temp.path().join("vault");
        let inbox = temp.path().join("received");
        for path in [&data, &vault, &inbox] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::create_dir(inbox.join("objects")).unwrap();
        std::fs::write(inbox.join("received-inbox.sqlite3"), b"fixture").unwrap();
        let original = temp.path().join("source.bin");
        std::fs::write(&original, b"verified receipt bytes").unwrap();
        let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
        core.ui_state
            .resolve_workspace(vault.to_str().unwrap(), 1)
            .unwrap();
        (temp, core, vault, inbox, original)
    }

    #[cfg(windows)]
    #[test]
    fn cannot_replace_existing_vault_file_or_remove_inbox_copy() {
        let (_temp, core, vault, inbox, original) = fixture();
        let bytes = b"verified receipt bytes";
        let mut stage = File::open(&original).unwrap();
        let digest = *blake3::hash(bytes).as_bytes();
        let id = Uuid::new_v4();
        let target = import_verified_inbox_as_new(
            &core,
            &inbox,
            &vault,
            &mut stage,
            bytes.len() as u64,
            digest,
            id,
        )
        .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        assert!(matches!(
            import_verified_inbox_as_new(
                &core,
                &inbox,
                &vault,
                &mut stage,
                bytes.len() as u64,
                digest,
                id,
            ),
            Err(FileCommandError::AlreadyExists { .. })
        ));
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
    }

    #[cfg(windows)]
    #[test]
    fn unknown_vault_or_modified_bytes_cannot_publish() {
        let (temp, core, vault, inbox, original) = fixture();
        let wrong = temp.path().join("unregistered");
        std::fs::create_dir(&wrong).unwrap();
        let mut stage = File::open(&original).unwrap();
        let id = Uuid::new_v4();
        let size = b"verified receipt bytes".len() as u64;
        let digest = *blake3::hash(b"verified receipt bytes").as_bytes();
        assert!(
            import_verified_inbox_as_new(&core, &inbox, &wrong, &mut stage, size, digest, id,)
                .is_err()
        );
        assert!(import_verified_inbox_as_new(
            &core,
            &inbox,
            &vault,
            &mut stage,
            size,
            *blake3::hash(b"corrupt").as_bytes(),
            id,
        )
        .is_err());
        assert!(!vault.join(format!("received-{id}.bin")).exists());
        assert_eq!(std::fs::read(original).unwrap(), b"verified receipt bytes");
    }

    #[cfg(not(windows))]
    #[test]
    fn other_platforms_are_fail_closed() {
        use crate::app_core::CoreEvent;
        use std::sync::Arc;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("app");
        std::fs::create_dir(&root).unwrap();
        let p = temp.path().join("source");
        std::fs::write(&p, b"bytes").unwrap();
        let mut stage = File::open(&p).unwrap();
        let core = Core::open(&root, Arc::new(|_: CoreEvent| {}));
        assert!(import_verified_inbox_as_new(
            &core,
            &root,
            &root,
            &mut stage,
            5,
            *blake3::hash(b"bytes").as_bytes(),
            Uuid::new_v4(),
        )
        .is_err());
        assert_eq!(std::fs::read(p).unwrap(), b"bytes");
    }
}
