// SYNC-CRASH-MASTER-01 / G2-01: deterministic I/O-error simulation AFTER
// the irreversible V7 attempt but BEFORE the single existing File Gate writer.
// These are test-only closure failures, NOT actual EIO/ENOSPC syscalls, power
// loss or evidence of a native durability barrier. No production fault hook.
#[test]
fn g2_prewrite_eio_and_enospc_hold_v7_and_never_publish() {
    use crate::files::error::FileCommandError;

    for fault in ["EIO", "ENOSPC"] {
        let f = fixture();
        let path = f.root.join(f.intent.path.as_str());
        let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
        let prewrite = Cell::new(0_usize);
        let postflight = Cell::new(false);

        let result = save_existing_note_with_prewrite_under_custody(
            &admission,
            &f.custody,
            f.keyspace.vault_id(),
            &f.scope,
            &f.intent.path,
            &f.new_text,
            &f.old_hash,
            Source::External,
            Some("g2-prewrite-io-fault-lab"),
            || prepare(&f, &admission),
            |prepared| {
                prewrite.set(prewrite.get() + 1);
                assert_editor_reserved(&f);
                prepared.inspect_before_write(
                    &admission, &f.bindings, &f.journal.borrow(), &f.protected,
                )?;
                // Test-only forced refusal under the held real File Gate
                // mutation lock; the actual primary writer is never called.
                Err(FileCommandError::Task {
                    message: format!("G2 simulated prewrite {fault}"),
                })
            },
            |_, _| {
                postflight.set(true);
                Ok(())
            },
        );
        assert!(matches!(result, Err(GuardedNoteWriteError::NeedsReview(_))),
            "{fault}: consumed V7 must be ambiguous HOLD, not success: {result:?}");
        assert_eq!(prewrite.get(), 1, "{fault}: prewrite must run exactly once");
        assert!(!postflight.get(), "{fault}: no postflight after the refused write");
        assert_eq!(fs::read(&path).unwrap(), b"unchanged private original",
            "{fault}: primary file must retain its original bytes");
        assert!(f.journal.borrow()
            .read_existing_replace_attempt(f.intent.operation_id)
            .unwrap().is_some(), "{fault}: one-shot V7 fence must survive");
        assert!(f.journal.borrow()
            .read_existing_replace_receipt_candidate(f.intent.operation_id)
            .unwrap().is_none(), "{fault}: no V8 can be minted");
        assert_reservation_released(&f);
        assert_no_remote_ack(&f);

        // An error after the irreversible claim is not retryable.
        let replay = f.custody.with_cooperative_writer_lifetime(
            &admission, f.keyspace.vault_id(), &f.scope,
            || prepare(&f, &admission),
        ).unwrap();
        assert!(replay.is_err(), "{fault}: a consumed V7 cannot be reissued");
        drop(replay);
        drop(admission);

        // Every writer handle and Core is closed before independent cold
        // inspection. The child's single existing oracle must return
        // PRIMARY_NOT_PUBLISHED/DENY, with NO receipt or S3 ACK authority.
        let Fixture {
            _temp: keep_fixture, core, root, appdata, bindings, custody,
            journal, cache, protected, intent, old_hash, ..
        } = f;
        drop(bindings);
        drop(custody);
        drop(journal);
        drop(cache);
        drop(protected);
        drop(core);
        let report = p0g3d09_run_oracle_child(
            &root, &appdata, intent.operation_id, "DENY", "PRIMARY_NOT_PUBLISHED",
        );
        assert_eq!(report["authority"], "none");
        assert_eq!(fs::read(root.join("Reserved.md")).unwrap(),
            b"unchanged private original");
        let mut cold = SqliteSyncJournal::open(&appdata.join("journal.sqlite")).unwrap();
        assert_eq!(cold.get(intent.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
        assert_eq!(cold.baseline(intent.vault_id, intent.document_id)
            .unwrap().unwrap().content_hash, old_hash);
        assert!(matches!(cold.mark_applied(intent.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)));
        drop(cold);
        drop(keep_fixture);
    }
}
