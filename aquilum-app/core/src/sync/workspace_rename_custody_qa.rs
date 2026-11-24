//! Negative QA for reusing File Gate's cooperative, old-root-bound OS lease.
//!
//! No production rename is authorized by this experiment. All filesystem
//! mutations occur in a disposable test root. The existing writer lease
//! rechecks the OLD root after its closure and cannot safely bless a move.
//! A different, explicitly transitioning host epoch is required.
use super::filegate_custody_identity::SqliteCustodyIdentityStore;
use super::filegate_lock_custody::scoped_leaf;
use super::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
use super::{RegisteredReadAdmission, VaultId};
use crate::app_core::{Core, CoreEvent};
use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::sync::Arc;

fn canonical_root(path: &Path) -> std::path::PathBuf {
    crate::search::paths::canonical_path(path)
}

#[test]
fn existing_cooperative_writer_lifetime_cannot_authorize_whole_root_rename() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("OwnedVault");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("keep.md"), "retained contents").unwrap();
    let root = canonical_root(&root);
    let destination = root.with_file_name("RenamedVault");
    let data = temp.path().join("core-data");
    fs::create_dir(&data).unwrap();
    let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();

    let root_id = PinnedProbeRoot::open(&root).unwrap().identity();
    let scope = temp.path().join("external-custody").join(scoped_leaf(root_id));
    fs::create_dir_all(&scope).unwrap();
    let lock_leaf = scope.join(LOCK_NAME);
    fs::write(&lock_leaf, b"fixture-only-stable-lock").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&scope, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&lock_leaf, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let scope = canonical_root(&scope);
    let mut receipt = SqliteCustodyIdentityStore::open(
        &admission, &data.join("custody.sqlite"),
    ).unwrap();
    let vault = VaultId::new();
    receipt.bind_observed(&admission, vault, &scope).unwrap();

    // Existing writer API works while the path and physical root stay fixed.
    let check = receipt.with_cooperative_writer_lifetime(
        &admission, vault, &scope, || fs::read_to_string(root.join("keep.md")).unwrap(),
    );
    assert_eq!(check.unwrap(), "retained contents");

    // A root rename invalidates the old-root postflight invariant.
    // Windows may refuse it earlier because another open handle blocks
    // DELETE; that too must NOT turn into a successful move authorization.
    let invoked = Cell::new(false);
    let result = receipt.with_cooperative_writer_lifetime(
        &admission, vault, &scope, || {
            invoked.set(true);
            fs::rename(&root, &destination)
        },
    );
    assert!(invoked.get(), "must exercise the lease closure, not only early refusal");
    assert!(!matches!(result, Ok(Ok(()))),
        "old-root-bound writer lifetime must not authorize a whole-vault move");
    assert_eq!(fs::read(&lock_leaf).unwrap(), b"fixture-only-stable-lock");
    match result {
        Err(_) => {
            // After a committed move the old-root postflight refuses.
            // That refusal must never be misread as proof of rollback.
            assert!(!root.exists());
            assert_eq!(fs::read_to_string(destination.join("keep.md")).unwrap(),
                "retained contents");
        }
        Ok(Err(_)) => {
            // OS could not move the live root; nothing was overwritten.
            assert!(root.join("keep.md").is_file());
            assert!(!destination.exists());
        }
        Ok(Ok(())) => unreachable!(),
    }
}
