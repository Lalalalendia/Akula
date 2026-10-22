// G1-02b: typed refusal evidence from the existing verifier in a NEW OS process.
// These tests do not exercise remote I/O or grant any S3 apply permission.

#[test]
fn g1_02b_cold_oracle_distinguishes_lost_v7_and_protected_original() {
    for domain in ["v7", "original"] {
        let f = fixture();
        claim_cold_attempt_without_publication(
            &f, &RegisteredReadAdmission::open(&f.core, &f.root).unwrap(),
        );
        assert_no_remote_ack(&f);
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
        match domain {
            "v7" => {
                let db = rusqlite::Connection::open(appdata.join("journal.sqlite")).unwrap();
                assert_eq!(db.execute(
                    "DELETE FROM sync_existing_replace_attempts WHERE operation_id=?1",
                    [intent.operation_id.to_string()],
                ).unwrap(), 1);
            }
            "original" => {
                let db = rusqlite::Connection::open(appdata.join("originals.sqlite")).unwrap();
                assert_eq!(db.execute(
                    "DELETE FROM bound_preimages WHERE operation_id=?1",
                    [intent.operation_id.to_string()],
                ).unwrap(), 1);
                assert_eq!(db.execute(
                    "DELETE FROM protected_originals WHERE operation_id=?1",
                    [intent.operation_id.to_string()],
                ).unwrap(), 1);
            }
            _ => unreachable!(),
        }
        let expected = if domain == "v7" {
            "V7_UNAVAILABLE"
        } else {
            "PROTECTED_ORIGINAL_UNAVAILABLE"
        };
        let report = p0g3d09_run_oracle_child(
            &root, &appdata, intent.operation_id, "DENY", expected,
        );
        assert_eq!(report["authority"], "none");
        assert_eq!(std::fs::read(root.join("Reserved.md")).unwrap(),
            b"unchanged private original");
        let mut cold = SqliteSyncJournal::open(&appdata.join("journal.sqlite")).unwrap();
        let staged = cold.get(intent.operation_id).unwrap().unwrap();
        assert_eq!(staged.phase, JournalPhase::Staged);
        assert_eq!(cold.baseline(intent.vault_id, intent.document_id)
            .unwrap().unwrap().content_hash, old_hash);
        assert!(matches!(cold.mark_applied(intent.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)));
        drop(cold);
        drop(keep_fixture);
    }
}
