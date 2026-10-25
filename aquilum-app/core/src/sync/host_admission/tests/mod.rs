    use super::*;
    use crate::app_core::{Core, CoreEvent};
    use std::sync::Arc;

    fn fixture() -> (tempfile::TempDir, Arc<Core>, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        fs::create_dir(&root).unwrap();
        let appdata = temp.path().join("appdata");
        fs::create_dir(&appdata).unwrap();
        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        (temp, core, root)
    }

    fn scoped_anchor_fixture(temp: &tempfile::TempDir, root: &Path) -> PathBuf {
        let pin = super::super::filegate_lock_probe::PinnedProbeRoot::open(root).unwrap();
        let anchor = temp.path()
            .join("external-host-locks")
            .join(super::super::filegate_lock_custody::scoped_leaf(pin.identity()));
        fs::create_dir_all(&anchor).unwrap();
        let lock = anchor.join(super::super::filegate_lock_probe::LOCK_NAME);
        fs::write(&lock, b"forensic scope bytes").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&anchor, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        }
        // macOS /var may itself be a symlink to /private/var.
        // On Windows keep the ordinary absolute name so icacls can inspect
        // the fixture without depending on its support for verbatim paths.
        #[cfg(unix)]
        { fs::canonicalize(anchor).unwrap() }
        #[cfg(windows)]
        { anchor }
    }

    #[test]
    fn host_admission_external_scope_shared_across_independent_core_registries() {
        let (temp, core, root) = fixture();
        let scoped = scoped_anchor_fixture(&temp, &root);
        let lock = scoped.join(super::super::filegate_lock_probe::LOCK_NAME);
        let first = RegisteredReadAdmission::open(&core, &root).unwrap();
        let observed = first.inspect_external_filegate_lock(&scoped).unwrap();
        assert_ne!(observed.registered_root, observed.custody_scope);
        assert_ne!(observed.registered_root, observed.lock_object);

        let db_path = temp.path().join("other-core-data");
        fs::create_dir(&db_path).unwrap();
        let second_core = Core::open(&db_path, Arc::new(|_: CoreEvent| {}));
        assert!(matches!(
            RegisteredReadAdmission::open(&second_core, &root),
            Err(VaultPortError::PermissionDenied)
        ));
        second_core.ui_state.resolve_workspace(root.to_str().unwrap(), 2).unwrap();
        let second = RegisteredReadAdmission::open(&second_core, &root).unwrap();
        assert_ne!(first.registered_workspace_id(), second.registered_workspace_id());
        let again = second.inspect_external_filegate_lock(&scoped).unwrap();
        assert_eq!(observed, again, "separate Core DB must find identical OS lock");
        assert_eq!(fs::read(&lock).unwrap(), b"forensic scope bytes");
        assert!(!root.join(super::super::filegate_lock_probe::LOCK_NAME).exists());
    }

    #[test]
    fn host_admission_external_scope_denies_missing_lock_bad_scope_and_nested_vault() {
        let (temp, core, root) = fixture();
        let scoped = scoped_anchor_fixture(&temp, &root);
        let lock = scoped.join(super::super::filegate_lock_probe::LOCK_NAME);
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        fs::remove_file(&lock).unwrap();
        assert!(admission.inspect_external_filegate_lock(&scoped).is_err());
        assert!(!lock.exists(), "observation never creates or repairs locks");

        fs::write(&lock, b"unchanged").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let wrong = temp.path().join("external-host-locks").join("vault-unrelated");
        fs::create_dir(&wrong).unwrap();
        assert!(admission.inspect_external_filegate_lock(&wrong).is_err());

        let nested = root.join(super::super::filegate_lock_custody::scoped_leaf(
            super::super::filegate_lock_probe::PinnedProbeRoot::open(&root).unwrap().identity()
        ));
        fs::create_dir(&nested).unwrap();
        assert!(admission.inspect_external_filegate_lock(&nested).is_err());
        assert_eq!(fs::read(lock).unwrap(), b"unchanged");
    }

    #[cfg(unix)]
    #[test]
    fn host_admission_external_scope_requires_private_owner_and_file_modes() {
        use std::os::unix::fs::PermissionsExt;
        let (temp, core, root) = fixture();
        let scoped = scoped_anchor_fixture(&temp, &root);
        let lock = scoped.join(super::super::filegate_lock_probe::LOCK_NAME);
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        assert!(admission.inspect_external_filegate_lock(&scoped).is_ok());

        fs::set_permissions(&scoped, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(admission.inspect_external_filegate_lock(&scoped).is_err());
        fs::set_permissions(&scoped, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(admission.inspect_external_filegate_lock(&scoped).is_err());
        fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(admission.inspect_external_filegate_lock(&scoped).is_ok());
    }


    #[cfg(unix)]
    #[test]
    fn host_admission_external_scope_refuses_symlinked_parent() {
        use std::os::unix::fs::symlink;
        let (temp, core, root) = fixture();
        let scoped = scoped_anchor_fixture(&temp, &root);
        let base = fs::canonicalize(temp.path()).unwrap();
        let alias = base.join("redirect-parent");
        symlink(scoped.parent().unwrap(), &alias).unwrap();
        let path = alias.join(scoped.file_name().unwrap());
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        assert!(admission.inspect_external_filegate_lock(&path).is_err());
        assert!(admission.inspect_external_filegate_lock(&scoped).is_ok());
    }

    #[test]
    fn host_admission_external_scope_refuses_newly_registered_overlap_during_lock() {
        use std::cell::Cell;
        let (temp, _core, root) = fixture();
        let scoped = scoped_anchor_fixture(&temp, &root);
        let id = super::super::filegate_lock_probe::PinnedProbeRoot::open(&root)
            .unwrap().identity();
        let calls = Cell::new(0usize);
        // A second registered workspace begins overlapping AFTER the first
        // registry snapshot. The inspector must query registration AGAIN
        // while holding the OS lock, not reuse the original list.
        let outcome = super::super::filegate_lock_custody::inspect_external_scope(
            &root,
            id,
            || {
                let current = calls.get();
                calls.set(current + 1);
                let mut roots = vec![root.clone()];
                if current > 0 { roots.push(scoped.clone()); }
                Ok(roots)
            },
            &scoped,
            || Ok(()),
        );
        assert!(outcome.is_err());
        assert!(calls.get() >= 2, "must re-read the registry while locked");
        assert_eq!(
            fs::read(scoped.join(super::super::filegate_lock_probe::LOCK_NAME)).unwrap(),
            b"forensic scope bytes"
        );
    }

    #[cfg(windows)]
    #[test]
    fn host_admission_external_scope_windows_refuses_everyone_write_ace() {
        use std::process::Command;
        let (temp, core, root) = fixture();
        let scoped = scoped_anchor_fixture(&temp, &root);
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        assert!(admission.inspect_external_filegate_lock(&scoped).is_ok());

        // All changes affect only a disposable test fixture. A deliberately
        // granted WORLD write ACE must invalidate the external custody scope.
        let command = Command::new("icacls")
            .arg(&scoped).arg("/grant").arg("*S-1-1-0:(F)")
            .output().unwrap();
        assert!(command.status.success(), "icacls: {}", String::from_utf8_lossy(&command.stderr));
        assert!(admission.inspect_external_filegate_lock(&scoped).is_err());

        // Even with a safe parent, a broad ACL on the lock LEAF must refuse.
        let clean = scoped_anchor_fixture(&temp, &root);
        let file = clean.join(super::super::filegate_lock_probe::LOCK_NAME);
        // Undo the test-only scope ACE before checking the leaf independently.
        let reset = Command::new("icacls")
            .arg(&scoped).arg("/remove:g").arg("*S-1-1-0")
            .output().unwrap();
        assert!(reset.status.success());
        assert!(admission.inspect_external_filegate_lock(&clean).is_ok());
        let granted = Command::new("icacls")
            .arg(&file).arg("/grant").arg("*S-1-1-0:(F)")
            .output().unwrap();
        assert!(granted.status.success());
        assert!(admission.inspect_external_filegate_lock(&clean).is_err());
    }

    fn custody_receipt_db(temp: &tempfile::TempDir, label: &str) -> PathBuf {
        let data = temp.path().join(label);
        fs::create_dir_all(&data).unwrap();
        data.join("custody-identity.sqlite")
    }

    #[test]
    fn host_admission_custody_receipt_survives_two_core_restarts_with_one_physical_anchor() {
        use super::super::{filegate_custody_identity::SqliteCustodyIdentityStore, VaultId};
        let (temp, first_core, root) = fixture();
        let second_data = temp.path().join("independent-core");
        fs::create_dir(&second_data).unwrap();
        let second_core = Core::open(&second_data, Arc::new(|_: CoreEvent| {}));
        second_core.ui_state.resolve_workspace(root.to_str().unwrap(), 2).unwrap();
        let first_db = custody_receipt_db(&temp, "receipt-a");
        let second_db = custody_receipt_db(&temp, "receipt-b");
        let anchor = scoped_anchor_fixture(&temp, &root);
        let shared_vault = VaultId::new();
        let first_registration = first_core.ui_state.list_workspaces().unwrap()[0].id;
        let second_registration = second_core.ui_state.list_workspaces().unwrap()[0].id;
        assert_ne!(first_registration, second_registration);
        let first_record;
        {
            let a = RegisteredReadAdmission::open(&first_core, &root).unwrap();
            let b = RegisteredReadAdmission::open(&second_core, &root).unwrap();
            let mut a_store = SqliteCustodyIdentityStore::open(&a, &first_db).unwrap();
            let mut b_store = SqliteCustodyIdentityStore::open(&b, &second_db).unwrap();
            first_record = a_store.bind_observed(&a, shared_vault, &anchor).unwrap();
            let second_record = b_store.bind_observed(&b, shared_vault, &anchor).unwrap();
            assert_ne!(first_record.workspace_id, second_record.workspace_id);
            assert_eq!(first_record.vault_id, second_record.vault_id);
            assert_eq!(first_record.registered_root_file, second_record.registered_root_file);
            assert_eq!(first_record.custody_scope_file, second_record.custody_scope_file);
            assert_eq!(first_record.lock_file, second_record.lock_file);
            assert_eq!(a_store.bind_observed(&a, shared_vault, &anchor).unwrap(), first_record);
            assert_eq!(b_store.bind_observed(&b, shared_vault, &anchor).unwrap(), second_record);
        }
        drop(first_core);
        drop(second_core);
        let reopened_a = Core::open(&temp.path().join("appdata"), Arc::new(|_: CoreEvent| {}));
        let reopened_b = Core::open(&second_data, Arc::new(|_: CoreEvent| {}));
        for (core, db, id) in [
            (&reopened_a, &first_db, first_registration),
            (&reopened_b, &second_db, second_registration),
        ] {
            let admission = RegisteredReadAdmission::open(core, &root).unwrap();
            let store = SqliteCustodyIdentityStore::open(&admission, db).unwrap();
            let verified = store.verify_observed(&admission, shared_vault, &anchor).unwrap();
            assert_eq!(verified.workspace_id, id);
            assert_eq!(verified.lock_file, first_record.lock_file);
            assert_eq!(verified.custody_scope_file, first_record.custody_scope_file);
        }
        assert_eq!(
            fs::read(anchor.join(super::super::filegate_lock_probe::LOCK_NAME)).unwrap(),
            b"forensic scope bytes"
        );
    }

    #[test]
    fn host_admission_custody_receipt_refuses_lock_substitution_without_rebinding() {
        use super::super::{filegate_custody_identity::SqliteCustodyIdentityStore, VaultId};
        let (temp, core, root) = fixture();
        let anchor = scoped_anchor_fixture(&temp, &root);
        let lock = anchor.join(super::super::filegate_lock_probe::LOCK_NAME);
        let old = anchor.join("original-lock-safely-retained");
        let database = custody_receipt_db(&temp, "receipt-c");
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = SqliteCustodyIdentityStore::open(&admission, &database).unwrap();
        let vault = VaultId::new();
        let accepted = store.bind_observed(&admission, vault, &anchor).unwrap();
        fs::rename(&lock, &old).unwrap();
        fs::write(&lock, b"substitute").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(store.verify_observed(&admission, vault, &anchor).is_err());
        assert!(store.bind_observed(&admission, vault, &anchor).is_err());
        assert_eq!(fs::read(&old).unwrap(), b"forensic scope bytes");
        assert_eq!(fs::read(&lock).unwrap(), b"substitute");
        let substituted = anchor.join("substitute-retained");
        fs::rename(&lock, &substituted).unwrap();
        fs::rename(&old, &lock).unwrap();
        assert_eq!(store.verify_observed(&admission, vault, &anchor).unwrap(), accepted);
    }

    #[test]
    fn host_admission_custody_receipt_refuses_corruption_and_vault_id_rebinding() {
        use super::super::{
            filegate_custody_identity::{CustodyBindingError, SqliteCustodyIdentityStore},
            VaultId,
        };
        let (temp, core, root) = fixture();
        let anchor = scoped_anchor_fixture(&temp, &root);
        let database = custody_receipt_db(&temp, "receipt-d");
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = SqliteCustodyIdentityStore::open(&admission, &database).unwrap();
        let vault = VaultId::new();
        store.bind_observed(&admission, vault, &anchor).unwrap();
        assert!(matches!(
            store.bind_observed(&admission, VaultId::new(), &anchor),
            Err(CustodyBindingError::ConflictingIdentity)
        ));
        let independent = rusqlite::Connection::open(&database).unwrap();
        independent.execute(
            "UPDATE custody_anchor_binding SET record_hash = 'invalid' WHERE workspace_id = ?1",
            [admission.registered_workspace_id().to_string()],
        ).unwrap();
        assert!(matches!(
            store.verify_observed(&admission, vault, &anchor),
            Err(CustodyBindingError::CorruptedBinding)
        ));
        assert!(matches!(
            store.bind_observed(&admission, vault, &anchor),
            Err(CustodyBindingError::CorruptedBinding)
        ));
        let still_bad: String = independent.query_row(
            "SELECT record_hash FROM custody_anchor_binding LIMIT 1", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(still_bad, "invalid", "read must never auto-repair a bad record");
    }

    #[test]
    fn host_admission_custody_receipt_refuses_root_replacement_even_after_reopen() {
        use super::super::{filegate_custody_identity::SqliteCustodyIdentityStore, VaultId};
        let (temp, core, root) = fixture();
        let anchor = scoped_anchor_fixture(&temp, &root);
        let database = custody_receipt_db(&temp, "receipt-e");
        let vault = VaultId::new();
        {
            let original = RegisteredReadAdmission::open(&core, &root).unwrap();
            let mut store = SqliteCustodyIdentityStore::open(&original, &database).unwrap();
            store.bind_observed(&original, vault, &anchor).unwrap();
        }
        let moved = temp.path().join("original-vault-retained");
        fs::rename(&root, &moved).unwrap();
        fs::create_dir(&root).unwrap();
        let reopened = RegisteredReadAdmission::open(&core, &root).unwrap();
        let store = SqliteCustodyIdentityStore::open(&reopened, &database).unwrap();
        assert!(store.verify_observed(&reopened, vault, &anchor).is_err());
        assert!(root.read_dir().unwrap().next().is_none());
        assert!(moved.is_dir());
    }

    #[test]
    fn host_admission_external_scope_revocation_does_not_create_any_lock() {
        let (temp, core, root) = fixture();
        let scoped = scoped_anchor_fixture(&temp, &root);
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        let old = core.ui_state.list_workspaces().unwrap()[0].id;
        core.ui_state.forget_workspace(old).unwrap();
        assert!(admission.inspect_external_filegate_lock(&scoped).is_err());
        assert_eq!(
            fs::read(scoped.join(super::super::filegate_lock_probe::LOCK_NAME)).unwrap(),
            b"forensic scope bytes"
        );
    }

    fn note() -> SyncRelativePath {
        SyncRelativePath::parse("notes/caf\u{e9}.md").unwrap()
    }

    #[test]
    fn host_admission_registry_lock_observation_requires_exact_registered_root() {
        let (temp, core, root) = fixture();
        let lock = root.join(super::super::filegate_lock_probe::LOCK_NAME);
        fs::write(&lock, b"unchanged forensic fixture").unwrap();
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        let evidence = admission.inspect_existing_filegate_lock().unwrap();
        assert_eq!(
            evidence.workspace_id,
            core.ui_state.list_workspaces().unwrap()[0].id
        );
        assert_ne!(evidence.root_identity.1, 0);
        assert_ne!(evidence.lock_identity.1, 0);
        assert_eq!(fs::read(&lock).unwrap(), b"unchanged forensic fixture");

        // Separate Core and SQLite database must register the root itself.
        let other_data = temp.path().join("other-data");
        fs::create_dir(&other_data).unwrap();
        let other = Core::open(&other_data, Arc::new(|_: CoreEvent| {}));
        assert!(matches!(
            RegisteredReadAdmission::open(&other, &root),
            Err(VaultPortError::PermissionDenied)
        ));
        other.ui_state.resolve_workspace(root.to_str().unwrap(), 2).unwrap();
        let independent = RegisteredReadAdmission::open(&other, &root).unwrap();
        let independent_evidence = independent.inspect_existing_filegate_lock().unwrap();
        assert_ne!(evidence.workspace_id, independent_evidence.workspace_id);
        assert_eq!(evidence.root_identity, independent_evidence.root_identity);
        assert_eq!(evidence.lock_identity, independent_evidence.lock_identity);
        assert_eq!(fs::read(&lock).unwrap(), b"unchanged forensic fixture");
    }

    #[test]
    fn host_admission_registry_lock_missing_and_hardlinked_fail_closed() {
        let (_temp, core, root) = fixture();
        let lock = root.join(super::super::filegate_lock_probe::LOCK_NAME);
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        assert!(admission.inspect_existing_filegate_lock().is_err());
        assert!(!lock.exists(), "forensic admission must never create locks");

        fs::write(&lock, b"keep").unwrap();
        assert!(admission.inspect_existing_filegate_lock().is_ok());
        let alias = root.join("hardlink-alias");
        fs::hard_link(&lock, &alias).unwrap();
        assert!(admission.inspect_existing_filegate_lock().is_err());
        assert_eq!(fs::read(&lock).unwrap(), b"keep");
        assert_eq!(fs::read(alias).unwrap(), b"keep");
    }

    #[test]
    fn host_admission_registry_lock_revocation_and_root_swap_fail_closed() {
        let (temp, core, root) = fixture();
        let lock = root.join(super::super::filegate_lock_probe::LOCK_NAME);
        fs::write(&lock, b"original").unwrap();
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        assert!(admission.inspect_existing_filegate_lock().is_ok());

        let workspace_id = core.ui_state.list_workspaces().unwrap()[0].id;
        core.ui_state.forget_workspace(workspace_id).unwrap();
        assert!(matches!(
            admission.inspect_existing_filegate_lock(),
            Err(VaultPortError::PermissionDenied)
        ));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 3).unwrap();
        let refreshed = RegisteredReadAdmission::open(&core, &root).unwrap();

        let moved = temp.path().join("moved-original");
        fs::rename(&root, &moved).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(&lock, b"substituted").unwrap();
        assert!(refreshed.inspect_existing_filegate_lock().is_err());
        assert_eq!(fs::read(&lock).unwrap(), b"substituted");
        assert_eq!(
            fs::read(moved.join(super::super::filegate_lock_probe::LOCK_NAME)).unwrap(),
            b"original"
        );
    }

    #[test]
    fn host_admission_reads_exact_unicode_bytes_without_writing() {
        let (_temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let bytes = "Русский \u{1f600}\n".as_bytes();
        let path = root.join(note().as_str());
        fs::write(&path, bytes).unwrap();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let snapshot = guard.read_closed_preflight(&note(), 1024).unwrap();
        assert_eq!(snapshot.bytes, bytes);
        assert_eq!(snapshot.content_hash, blake3::hash(bytes).to_hex().to_string());
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert!(guard.read_closed_preflight(&note(), 0).is_err());
        assert!(guard.read_closed_preflight(&note(), (bytes.len() - 1) as u64).is_err());
    }

    #[test]
    fn host_admission_refuses_unregistered_root_and_revoked_registration() {
        let (_temp, core, root) = fixture();
        let unregistered = tempfile::tempdir().unwrap();
        assert!(matches!(
            RegisteredReadAdmission::open(&core, unregistered.path()),
            Err(VaultPortError::PermissionDenied)
        ));
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let id = core.ui_state.list_workspaces().unwrap()[0].id;
        core.ui_state.forget_workspace(id).unwrap();
        assert!(matches!(
            guard.read_closed_preflight(&note(), 1024),
            Err(VaultPortError::PermissionDenied)
        ));
    }

    #[test]
    fn host_admission_blocks_open_editors_and_concurrent_reservations() {
        let (_temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let path = root.join(note().as_str());
        fs::write(&path, b"unchanged").unwrap();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();

        core.documents.open(&core, &path, 0).unwrap();
        assert!(matches!(
            guard.read_closed_preflight(&note(), 1024),
            Err(VaultPortError::DirtyEditorBuffer)
        ));
        core.documents.release(&core, &path);

        let held = core.documents.reserve_sync_closed_document(&path).unwrap();
        assert!(matches!(
            guard.read_closed_preflight(&note(), 1024),
            Err(VaultPortError::PermissionDenied)
        ));
        drop(held);
        assert_eq!(guard.read_closed_preflight(&note(), 1024).unwrap().bytes, b"unchanged");
    }

    #[test]
    fn host_admission_rejects_root_replacement_instead_of_reading_either_tree() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join(note().as_str()), b"original").unwrap();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();

        fs::rename(&root, temp.path().join("old-vault")).unwrap();
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join(note().as_str()), b"substituted").unwrap();

        assert!(guard.read_closed_preflight(&note(), 1024).is_err());
        assert_eq!(fs::read(root.join(note().as_str())).unwrap(), b"substituted");
    }

    #[cfg(unix)]
    #[test]
    fn host_admission_rejects_symlinked_workspace_and_child() {
        use std::os::unix::fs::symlink;
        let (temp, core, root) = fixture();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("private.md"), b"secret").unwrap();
        symlink(outside.path(), root.join("jump")).unwrap();
        assert!(RegisteredReadAdmission::open(
            &core, &temp.path().join("alias")
        ).is_err());
        symlink(&root, temp.path().join("alias")).unwrap();
        assert!(RegisteredReadAdmission::open(
            &core, &temp.path().join("alias")
        ).is_err());
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        assert!(guard.read_closed_preflight(
            &SyncRelativePath::parse("jump/private.md").unwrap(), 1024
        ).is_err());
    }

    fn protection_store(temp: &tempfile::TempDir) -> SqliteProtectionStore {
        SqliteProtectionStore::open(&temp.path().join("appdata/protected.sqlite")).unwrap()
    }

    include!("preimage.rs");
