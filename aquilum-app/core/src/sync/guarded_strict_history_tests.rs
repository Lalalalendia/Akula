//! P0g-3d-03: genuine File Gate + immutable HistoryService witness tests.
//! This does NOT model a cold power cut, S3 apply or durable receipt.

use super::*;
use crate::app_core::{Core, CoreEvent};
use crate::history::service::HistoryFault;
use crate::sync::filegate_custody_identity::SqliteCustodyIdentityStore;
use crate::sync::filegate_lock_custody::scoped_leaf;
use crate::sync::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;

struct Fixture {
    _temp: tempfile::TempDir,
    core: Arc<Core>,
    root: PathBuf,
    note: PathBuf,
    scope: PathBuf,
    custody: SqliteCustodyIdentityStore,
    vault: VaultId,
    relative: SyncRelativePath,
    old_hash: String,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("appdata");
    let selected_root = temp.path().join("vault");
    fs::create_dir_all(&selected_root).unwrap();
    fs::create_dir_all(&data).unwrap();
    let root = crate::search::paths::canonical_path(&selected_root);
    let note = root.join("Strict.md");
    fs::write(&note, b"strict protected original").unwrap();
    let physical = PinnedProbeRoot::open(&root).unwrap().identity();
    let scope = temp.path().join("external").join(scoped_leaf(physical));
    fs::create_dir_all(&scope).unwrap();
    fs::write(scope.join(LOCK_NAME), b"existing cooperative lock").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&scope, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(scope.join(LOCK_NAME), fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[cfg(unix)]
    let scope = fs::canonicalize(scope).unwrap();

    let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    let mut custody = SqliteCustodyIdentityStore::open(
        &admission, &data.join("custody.sqlite"),
    ).unwrap();
    let vault = VaultId::new();
    custody.bind_observed(&admission, vault, &scope).unwrap();
    Fixture {
        _temp: temp,
        core, root, note, scope, custody, vault,
        relative: SyncRelativePath::parse("Strict.md").unwrap(),
        old_hash: blake3::hash(b"strict protected original").to_hex().to_string(),
    }
}

#[test]
fn strict_history_witness_is_exact_new_external_version_under_one_filegate() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let content = "an immutable remote candidate ".repeat(100);
    let expected = blake3::hash(content.as_bytes()).to_hex().to_string();
    let prewrite_called = Cell::new(0);
    let postflight_called = Cell::new(0);
    let observed = save_existing_note_with_prewrite_strict_history_under_custody(
        &admission, &f.custody, f.vault, &f.scope, &f.relative,
        &content, &f.old_hash, Source::External, Some("strict-test"),
        || {
            // Reservation is retained through both HistoryService and postflight.
            f.core.documents.reserve_sync_closed_document(&f.note)
                .map_err(|error| FileCommandError::Task {
                    message: format!("reservation refused: {error:?}"),
                })
        },
        |_| {
            prewrite_called.set(prewrite_called.get() + 1);
            assert!(matches!(f.core.file_mutations.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)),
                "strict prewrite must execute inside the single real File Gate");
            Ok(())
        },
        |_, primary, history| {
            postflight_called.set(postflight_called.get() + 1);
            assert_eq!(primary.hash, expected);
            assert_eq!(history.content_hash(), expected);
            assert_eq!(history.content_length(), content.len());
            assert!(!history.version_id().is_empty());
            assert!(matches!(f.core.documents.open(&f.core, &f.note, 0),
                Err(crate::documents::hub::HubError::SyncReserved)));
            Ok(())
        },
    ).unwrap();
    assert_eq!(observed.primary.hash, expected);
    assert_eq!(observed.history.content_hash(), expected);
    assert_eq!(observed.history.content_length(), content.len());
    assert_eq!(prewrite_called.get(), 1);
    assert_eq!(postflight_called.get(), 1);
    assert_eq!(fs::read_to_string(&f.note).unwrap(), content);
    assert_eq!(fs::read(f.scope.join(LOCK_NAME)).unwrap(),
        b"existing cooperative lock");
    assert!(f.core.history.page(&|| vec![f.root.clone()], &f.note, 0, 8).total > 0);
    assert!(f.core.documents.reserve_sync_closed_document(&f.note).is_ok());
}

#[test]
fn strict_history_invalid_source_denied_before_claim_or_file_mutation() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let prepared = Cell::new(false);
    let prewrite = Cell::new(false);
    let result = save_existing_note_with_prewrite_strict_history_under_custody(
        &admission, &f.custody, f.vault, &f.scope, &f.relative,
        "forbidden local typing", &f.old_hash, Source::Me, None,
        || { prepared.set(true); Ok(()) },
        |_| { prewrite.set(true); Ok(()) },
        |_, _, _| Ok(()),
    );
    assert!(matches!(result, Err(GuardedNoteWriteError::Denied(_))));
    assert!(!prepared.get());
    assert!(!prewrite.get());
    assert_eq!(fs::read(&f.note).unwrap(), b"strict protected original");
}

#[test]
fn strict_history_injected_failure_after_primary_is_needs_review_not_receipt() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let content = "new remote update with failed history ".repeat(50);
    f.core.history.fail_once(HistoryFault::CreateVersion);
    let postflight = Cell::new(false);
    let outcome = save_existing_note_with_prewrite_strict_history_under_custody(
        &admission, &f.custody, f.vault, &f.scope, &f.relative,
        &content, &f.old_hash, Source::External, None,
        || Ok(()), |_| Ok(()),
        |_, _, _| { postflight.set(true); Ok(()) },
    );
    assert!(matches!(outcome, Err(GuardedNoteWriteError::NeedsReview(_))));
    assert!(!postflight.get(), "failed history must not permit receipt inspection");
    assert_eq!(fs::read_to_string(&f.note).unwrap(), content,
        "primary may already have changed: no retry or baseline promotion");
}

#[test]
fn strict_history_rejects_wrong_core_before_prewrite_or_primary_mutation() {
    let f = fixture();
    let foreign_data = f._temp.path().join("foreign-data");
    fs::create_dir_all(&foreign_data).unwrap();
    let foreign = Core::open(&foreign_data, Arc::new(|_: CoreEvent| {}));
    let permit = HeldCustodyHistoryPermit { core: &f.core, note: &f.note };
    let mut prewrite = false;
    let result = gate::write_checked_history_with_prewrite_verified(
        &permit, &foreign, &f.note, "illegal", &f.old_hash,
        Source::External, None,
        || { prewrite = true; Ok(()) },
    );
    assert!(matches!(result, Err(CheckedHistoryError::Primary(_))));
    assert!(!prewrite);
    assert_eq!(fs::read(&f.note).unwrap(), b"strict protected original");
}
