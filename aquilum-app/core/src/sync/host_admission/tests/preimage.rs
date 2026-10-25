    #[test]
    fn live_preimage_survives_database_restart_with_bound_root_and_file_identity() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let bytes = "Глава 140. Русский текст 😀".as_bytes();
        let note = note();
        fs::write(root.join(note.as_str()), bytes).unwrap();
        let hash = blake3::hash(bytes).to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let original_root = guard.initial;
        let file_id = guard.pinned.read_identified_bounded(
            &note, MAX_PROTECTED_ORIGINAL_BYTES as u64).unwrap().identity.unwrap();
        {
            let mut store = protection_store(&temp);
            assert_eq!(
                guard.preserve_closed_original(&note, &hash, operation, &mut store).unwrap(),
                ProtectionOutcome::Stored
            );
            assert_eq!(
                guard.preserve_closed_original(&note, &hash, operation, &mut store).unwrap(),
                ProtectionOutcome::AlreadyStored
            );
        }
        let store = protection_store(&temp);
        let saved = store.read_bound_preimage(operation).unwrap().unwrap();
        assert_eq!(saved.original.bytes, bytes);
        assert_eq!(saved.original.path, note);
        assert_eq!(saved.original.content_hash, hash);
        assert_eq!(saved.root.device, original_root.0);
        assert_eq!(saved.root.file, original_root.1);
        assert_eq!(saved.file, file_id);
        assert_eq!(fs::read(root.join(note.as_str())).unwrap(), bytes);
    }

    #[test]
    fn preimage_capture_rejects_unexpected_hash_or_open_editor_without_backup() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let note = note();
        let target = root.join(note.as_str());
        fs::write(&target, b"original").unwrap();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        let op = OperationId::new();

        assert!(matches!(
            guard.preserve_closed_original(&note, "wrong", op, &mut store),
            Err(BoundCaptureError::OriginalHashMismatch)
        ));
        core.documents.open(&core, &target, 0).unwrap();
        let digest = blake3::hash(b"original").to_hex().to_string();
        assert!(matches!(
            guard.preserve_closed_original(&note, &digest, op, &mut store),
            Err(BoundCaptureError::Admission(VaultPortError::DirtyEditorBuffer))
        ));
        core.documents.release(&core, &target);
        assert!(store.read_preimage(op).unwrap().is_none());
        assert_eq!(fs::read(target).unwrap(), b"original");
    }

    #[test]
    fn replacing_a_file_with_identical_bytes_does_not_preserve_old_file_identity() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let note = note();
        let target = root.join(note.as_str());
        fs::write(&target, b"same text").unwrap();
        let digest = blake3::hash(b"same text").to_hex().to_string();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        let id = OperationId::new();

        let result = guard.preserve_with_hook(&note, &digest, id, &mut store, || {
            fs::rename(&target, root.join("notes/moved.md")).unwrap();
            fs::write(&target, b"same text").unwrap();
        });
        assert!(matches!(
            result, Err(BoundCaptureError::Admission(VaultPortError::FileChanged))
        ));
        let saved = store.read_bound_preimage(id).unwrap().unwrap();
        assert_eq!(saved.original.bytes, b"same text");
        assert_eq!(fs::read(&target).unwrap(), b"same text");
        // Reusing the same operation ID after the file was swapped must not
        // silently confer the old file's recorded identity on a new inode.
        assert!(matches!(
            guard.preserve_closed_original(&note, &digest, id, &mut store),
            Err(BoundCaptureError::Protection(ProtectionStoreError::OperationCollision))
        ));
    }

    #[test]
    fn root_replacement_before_persistence_refuses_without_storing_a_backup() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join(note().as_str()), b"original").unwrap();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        let operation = OperationId::new();
        let digest = blake3::hash(b"original").to_hex().to_string();
        let result = guard.preserve_with_hook(&note(), &digest, operation, &mut store, || {
            fs::rename(&root, temp.path().join("moved-vault")).unwrap();
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join("notes")).unwrap();
            fs::write(root.join(note().as_str()), b"replacement").unwrap();
        });
        assert!(result.is_err());
        assert!(store.read_preimage(operation).unwrap().is_none());
        assert_eq!(fs::read(root.join(note().as_str())).unwrap(), b"replacement");
    }

    #[test]
    fn hardlink_alias_and_workspace_local_database_cannot_pass_capture() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let target = root.join(note().as_str());
        fs::write(&target, b"protected source").unwrap();
        fs::hard_link(&target, root.join("notes/alias.md")).unwrap();
        let digest = blake3::hash(b"protected source").to_hex().to_string();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut outside_store = protection_store(&temp);
        let id = OperationId::new();
        assert!(matches!(
            guard.preserve_closed_original(&note(), &digest, id, &mut outside_store),
            Err(BoundCaptureError::Admission(VaultPortError::UnsafePath))
        ));
        assert!(outside_store.read_preimage(id).unwrap().is_none());

        fs::remove_file(root.join("notes/alias.md")).unwrap();
        let mut inside_store = SqliteProtectionStore::open(&root.join("protection.sqlite")).unwrap();
        assert!(matches!(
            guard.preserve_closed_original(&note(), &digest, id, &mut inside_store),
            Err(BoundCaptureError::Admission(VaultPortError::PermissionDenied))
        ));
        assert!(inside_store.read_preimage(id).unwrap().is_none());
    }

    #[test]
    fn durable_preimage_revalidation_after_restart_is_read_only() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let path = root.join(note().as_str());
        fs::write(&path, b"stable original").unwrap();
        let expected = blake3::hash(b"stable original").to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        {
            let mut store = protection_store(&temp);
            guard.preserve_closed_original(&note(), &expected, operation, &mut store).unwrap();
        }
        let store = protection_store(&temp);
        guard.revalidate_bound_preimage(&note(), &expected, operation, &store).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"stable original");
    }

    #[test]
    fn durable_preimage_revalidation_rejects_same_byte_replacement() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let path = root.join(note().as_str());
        fs::write(&path, b"same content").unwrap();
        let expected = blake3::hash(b"same content").to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        guard.preserve_closed_original(&note(), &expected, operation, &mut store).unwrap();

        fs::rename(&path, root.join("notes/old-identity.md")).unwrap();
        fs::write(&path, b"same content").unwrap();
        assert!(matches!(
            guard.revalidate_bound_preimage(&note(), &expected, operation, &store),
            Err(BoundCaptureError::Admission(VaultPortError::FileChanged))
        ));
        assert_eq!(fs::read(&path).unwrap(), b"same content");
        assert_eq!(store.read_bound_preimage(operation).unwrap().unwrap().original.bytes,
            b"same content");
    }

    #[test]
    fn durable_preimage_revalidation_rejects_wrong_operation_and_open_editor() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let path = root.join(note().as_str());
        fs::write(&path, b"original").unwrap();
        let expected = blake3::hash(b"original").to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        guard.preserve_closed_original(&note(), &expected, operation, &mut store).unwrap();

        assert!(matches!(
            guard.revalidate_bound_preimage(&note(), &expected, OperationId::new(), &store),
            Err(BoundCaptureError::Protection(ProtectionStoreError::UnboundOriginal))
        ));
        assert!(matches!(
            guard.revalidate_bound_preimage(&note(), "wrong-hash", operation, &store),
            Err(BoundCaptureError::Protection(ProtectionStoreError::OperationCollision))
        ));
        core.documents.open(&core, &path, 0).unwrap();
        assert!(matches!(
            guard.revalidate_bound_preimage(&note(), &expected, operation, &store),
            Err(BoundCaptureError::Admission(VaultPortError::DirtyEditorBuffer))
        ));
        core.documents.release(&core, &path);
        assert_eq!(fs::read(path).unwrap(), b"original");
    }

    #[test]
    fn durable_preimage_revalidation_rejects_replaced_registered_root() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join(note().as_str()), b"original").unwrap();
        let expected = blake3::hash(b"original").to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        guard.preserve_closed_original(&note(), &expected, operation, &mut store).unwrap();

        let old_root = temp.path().join("old-vault");
        fs::rename(&root, &old_root).unwrap();
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join(note().as_str()), b"original").unwrap();
        assert!(guard.revalidate_bound_preimage(&note(), &expected, operation, &store).is_err());
        assert_eq!(fs::read(root.join(note().as_str())).unwrap(), b"original");
        assert_eq!(fs::read(old_root.join(note().as_str())).unwrap(), b"original");
    }


    // The current Rust test binary is also our minimal cross-platform child.
    // The parent passes paths through environment values so Unicode and spaces
    // are not parsed by a shell. The child's direct std::fs calls deliberately
    // DO NOT participate in Akula's process-wide Core File Gate.
    #[test]
    fn external_process_path_swap_fixture() {
        let Some(mode) = std::env::var_os("AKULA_SYNC_TEST_SWAP_MODE") else {
            return;
        };
        let target = PathBuf::from(std::env::var_os("AKULA_SYNC_TEST_TARGET").unwrap());
        let marker = PathBuf::from(std::env::var_os("AKULA_SYNC_TEST_MARKER").unwrap());
        match mode.to_str().unwrap() {
            "file" => {
                let moved = target.with_extension("saved-pre-swap");
                fs::rename(&target, &moved).unwrap();
                // Same bytes, different inode/NTFS file ID. Holding on to the
                // old renamed file prevents file ID reuse in this fixture.
                fs::write(&target, b"same bytes").unwrap();
            }
            #[cfg(unix)]
            "directory" => {
                use std::os::unix::fs::symlink;
                let moved = PathBuf::from(std::env::var_os("AKULA_SYNC_TEST_MOVED").unwrap());
                let outside = PathBuf::from(std::env::var_os("AKULA_SYNC_TEST_OUTSIDE").unwrap());
                fs::rename(&target, &moved).unwrap();
                symlink(&outside, &target).unwrap();
            }
            other => panic!("unsupported child operation: {other}"),
        }
        fs::write(marker, b"child-swapped").unwrap();
    }

    fn run_external_swap(
        mode: &str,
        target: &Path,
        marker: &Path,
        moved: Option<&Path>,
        outside: Option<&Path>,
    ) {
        use std::process::Command;
        let mut child = Command::new(std::env::current_exe().unwrap());
        child.arg("external_process_path_swap_fixture")
            .arg("--nocapture")
            .env("AKULA_SYNC_TEST_SWAP_MODE", mode)
            .env("AKULA_SYNC_TEST_TARGET", target)
            .env("AKULA_SYNC_TEST_MARKER", marker);
        if let Some(path) = moved {
            child.env("AKULA_SYNC_TEST_MOVED", path);
        }
        if let Some(path) = outside {
            child.env("AKULA_SYNC_TEST_OUTSIDE", path);
        }
        let status = child.status().unwrap();
        assert!(status.success(), "external test process failed: {status:?}");
        assert_eq!(fs::read(marker).unwrap(), b"child-swapped",
            "test runner must actually execute the child fixture");
    }

    #[test]
    fn host_admission_detects_foreign_process_swap_while_core_gate_is_held() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let relative = note();
        let target = root.join(relative.as_str());
        fs::write(&target, b"same bytes").unwrap();
        let digest = blake3::hash(b"same bytes").to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        guard.preserve_closed_original(&relative, &digest, operation, &mut store).unwrap();

        // An ordinary file editor in another process does not acquire this
        // mutex. This is a deterministic interprocess replacement, not a
        // claim that revalidation prevents swaps immediately after it returns.
        let held = core.file_mutations.lock().unwrap();
        let marker = temp.path().join("foreign-file-swapped");
        run_external_swap("file", &target, &marker, None, None);
        assert_eq!(fs::read(&target).unwrap(), b"same bytes");
        drop(held);

        assert!(matches!(
            guard.revalidate_bound_preimage(&relative, &digest, operation, &store),
            Err(BoundCaptureError::Admission(VaultPortError::FileChanged))
        ));
        assert_eq!(store.read_bound_preimage(operation).unwrap().unwrap().original.bytes,
            b"same bytes");
        assert_eq!(fs::read(target.with_extension("saved-pre-swap")).unwrap(),
            b"same bytes");
    }

    #[cfg(unix)]
    #[test]
    fn host_admission_detects_foreign_process_symlink_parent_swap() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let relative = note();
        fs::write(root.join(relative.as_str()), b"original").unwrap();
        let digest = blake3::hash(b"original").to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        guard.preserve_closed_original(&relative, &digest, operation, &mut store).unwrap();

        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("café.md"), b"external secret").unwrap();
        let moved = temp.path().join("old-notes");
        let marker = temp.path().join("foreign-directory-swapped");
        let held = core.file_mutations.lock().unwrap();
        run_external_swap("directory", &root.join("notes"), &marker, Some(&moved), Some(&outside));
        drop(held);

        assert!(guard.revalidate_bound_preimage(&relative, &digest, operation, &store).is_err());
        assert_eq!(fs::read(outside.join("café.md")).unwrap(), b"external secret");
        assert_eq!(fs::read(moved.join("café.md")).unwrap(), b"original");
        assert_eq!(store.read_bound_preimage(operation).unwrap().unwrap().original.bytes,
            b"original");
    }

    #[test]
    fn protected_preimage_capture_holds_core_file_gate_interlock() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join(note().as_str()), b"first").unwrap();
        let expected = blake3::hash(b"first").to_hex().to_string();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        let operation = OperationId::new();
        assert_eq!(
            guard.preserve_with_hook(&note(), &expected, operation, &mut store, || {
                assert!(
                    core.file_mutations.try_lock().is_err(),
                    "host provenance must hold the same Core mutex as ordinary Gate writes"
                );
            }).unwrap(),
            ProtectionOutcome::Stored
        );
        assert_eq!(store.read_bound_preimage(operation).unwrap().unwrap().original.bytes, b"first");
    }
