// S3-E2E-01: process-isolated MinIO receiver, never production activation.
// Included only from the core_stage test module.

    /// Separate, real OS process: re-open only the receiver's Core, registered
    /// workspace, SQLite state, pinned S3 snapshot and host-bound File Gate.
    /// The parent has already published one immutable MinIO revision but
    /// must NOT perform the child's local publication. No automatic Sync,
    /// confirmed baseline, ACK or overwrite is enabled by this test helper.
    #[tokio::test]
    async fn s3_e2e01_receiver_child_entrypoint() {
        use crate::app_core::{Core, CoreEvent};
        use crate::sync::filegate_custody_identity::SqliteCustodyIdentityStore;
        use crate::sync::host_admission::RegisteredReadAdmission;
        use crate::sync::profile_binding::SqliteRegisteredS3Bindings;
        use std::fs;
        use std::path::PathBuf;
        use std::sync::Arc;

        let Some(intent_json) = std::env::var_os("AKULA_S3_E2E01_CHILD_INTENT") else {
            return;
        };
        let intent: OperationIntent =
            serde_json::from_str(intent_json.to_str().unwrap()).unwrap();
        let root = PathBuf::from(std::env::var_os("AKULA_S3_E2E01_ROOT").unwrap());
        let appdata = PathBuf::from(std::env::var_os("AKULA_S3_E2E01_APPDATA").unwrap());
        let custody_scope =
            PathBuf::from(std::env::var_os("AKULA_S3_E2E01_CUSTODY_SCOPE").unwrap());
        let budget: u64 = std::env::var("AKULA_S3_E2E01_LIMIT").unwrap().parse().unwrap();
        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
        assert!(endpoint.starts_with("http://127.0.0.1:")
            || endpoint.starts_with("http://localhost:"),
            "the cross-process writer is restricted to disposable loopback MinIO");

        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        let ns = S3Keyspace::new("akula-sync/core-stage-ci", intent.vault_id).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-core-stage-test".into(),
            region: "us-east-1".into(),
            endpoint: Some(endpoint),
            force_path_style: true,
        };
        let receiver = AwsS3ProbeClient::connect(profile, ns).await.unwrap();
        let verified = receiver.materialize_verified_revision_metadata_store(256 * 1024)
            .await.unwrap().unwrap();
        let bindings = SqliteRegisteredS3Bindings::open(
            &admission, &appdata.join("s3-profiles.sqlite"),
        ).unwrap();
        let custody = SqliteCustodyIdentityStore::open(
            &admission, &appdata.join("s3-custody.sqlite"),
        ).unwrap();
        let mut journal = SqliteSyncJournal::open(&appdata.join(crate::sync::RECOVERY_JOURNAL_FILENAME)).unwrap();
        let mut cache = SqliteDagStore::open(&appdata.join("blobs.sqlite")).unwrap();

        let destination = root.join(intent.path.as_str());
        assert!(!destination.exists(), "receiver must start without this note");
        let created = receiver.receive_single_verified_s3_additive_once(
            &verified, &admission, &bindings, &custody, &custody_scope,
            &mut journal, &mut cache, intent.clone(), budget,
        ).await.unwrap();
        let bytes = fs::read(&destination).unwrap();
        assert_eq!(created.byte_length, bytes.len() as u64);
        assert_eq!(blake3::hash(&bytes).to_hex().to_string(),
            intent.expected_content_hash);
        assert!(matches!(
            journal.mark_applied(intent.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
        assert_eq!(journal.get(intent.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
        assert!(journal.baseline(intent.vault_id, intent.document_id).unwrap().is_none());
        assert_eq!(journal.local_publication_review_queue().unwrap().len(), 1);
        // This is the missing Core-to-Native review seam: shared cloud VaultId
        // is deliberately distinct from the machine-local workspace UUID.
        assert_eq!(
            crate::sync::native_recovery_overview(&core, &appdata, &root),
            crate::sync::NativeRecoveryOverview::NoLocalPublicationClaims,
        );
        match crate::sync::native_s3_bound_publication_overview(&core, &appdata, &root) {
            crate::sync::NativeRecoveryOverview::RequiresReview {
                total_claims, examples,
            } => {
                assert_eq!(total_claims, 1);
                assert_eq!(examples[0].operation_id, intent.operation_id.to_string());
                assert_eq!(examples[0].historical_path, intent.path.as_str());
            }
            other => panic!("remote S3 claim is hidden from Native review: {other:?}"),
        }
        println!("AKULA_S3_E2E01_CHILD=UNCONFIRMED_LOCAL_CREATE");
    }

    #[tokio::test]
    #[ignore = "requires isolated loopback MinIO"]
    async fn live_minio_two_os_processes_verified_additive_receive() {
        let mut bytes = b"# E2E-01\n\n".to_vec();
        bytes.resize(1024, b'x');
        verify_live_core_roundtrip_process(&bytes, bytes.len() as u64, true).await;
    }

    async fn verify_live_core_roundtrip_process(
        bytes: &[u8], max_blob_bytes: u64, receiver_in_child: bool,
    ) {
        use crate::app_core::{Core, CoreEvent};
        use crate::sync::{JournalError, RegisteredReadAdmission};
        use crate::sync::profile_binding::SqliteRegisteredS3Bindings;
        use crate::sync::filegate_custody_identity::SqliteCustodyIdentityStore;
        use crate::sync::filegate_lock_custody::scoped_leaf;
        use crate::sync::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
        use aws_sdk_s3::error::ProvideErrorMetadata;
        use std::fs;
        use std::sync::Arc;

        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
        assert!(
            endpoint.starts_with("http://127.0.0.1:")
                || endpoint.starts_with("http://localhost:")
        );
        let dir = tempfile::tempdir().unwrap();
        let root_a = dir.path().join("source");
        let root_b = dir.path().join("receiver");
        let app_a = dir.path().join("app-a");
        let app_b = dir.path().join("app-b");
        fs::create_dir_all(root_a.join("notes")).unwrap();
        fs::create_dir_all(root_b.join("notes")).unwrap();
        fs::create_dir_all(&app_a).unwrap();
        fs::create_dir_all(&app_b).unwrap();
        let first_core = Core::open(&app_a, Arc::new(|_: CoreEvent| {}));
        let second_core = Core::open(&app_b, Arc::new(|_: CoreEvent| {}));
        first_core.ui_state.resolve_workspace(root_a.to_str().unwrap(), 1).unwrap();
        second_core.ui_state.resolve_workspace(root_b.to_str().unwrap(), 2).unwrap();
        let source_id = first_core.ui_state.list_workspaces().unwrap()[0].id;
        let target_id = second_core.ui_state.list_workspaces().unwrap()[0].id;
        assert_ne!(source_id, target_id, "two independent local Core registrations");

        let relative = SyncRelativePath::parse("notes/remote.md").unwrap();
        fs::write(root_a.join(relative.as_str()), bytes).unwrap();
        // Only a read-only preflight for the source fixture; a production
        // uploader must separately prove its guarded mutation/source grant.
        let captured = RegisteredReadAdmission::open(&first_core, &root_a)
            .unwrap().read_closed_preflight(&relative, max_blob_bytes).unwrap();
        assert_eq!(captured.bytes, bytes);

        // The shared cloud UUID must differ from both independent machine-
        // local workspace registrations. No aliasing is permitted.
        let vault = VaultId::new();
        assert_ne!(vault.as_uuid(), source_id);
        assert_ne!(vault.as_uuid(), target_id);
        let ns = S3Keyspace::new("akula-sync/core-stage-ci", vault).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-core-stage-test".into(),
            region: "us-east-1".into(),
            endpoint: Some(endpoint),
            force_path_style: true,
        };
        let writer = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
        let reader = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
        // The MinIO workflow runs several live tests against one server.
        // A bucket already owned by these isolated test credentials is fine:
        // each fixture uses a fresh random vault keyspace. Any other S3
        // response must still fail rather than masking an authorization error.
        match writer.client.create_bucket().bucket("akula-core-stage-test").send().await {
            Ok(_) => {}
            Err(error) if error.as_service_error()
                .and_then(|service| service.code()) == Some("BucketAlreadyOwnedByYou") => {}
            Err(error) => panic!("test bucket provisioning failed: {error}"),
        }

        let blob_hash = captured.content_hash;
        let blob_key = ns.immutable_blob_key(&blob_hash).unwrap();
        if captured.bytes.len() as u64 <= super::super::MAX_PROBE_BYTES {
            writer.put_conditionally(&blob_key, &captured.bytes, S3PutCondition::IfAbsent)
                .await.unwrap();
        } else {
            // Test fixture provisioning only: exercise the PRODUCTION large
            // download path below, not the already separately tested multipart
            // uploader. The canonical immutable BLAKE3 key remains unchanged.
            writer.client.put_object().bucket(&profile.bucket).key(&blob_key)
                .if_none_match("*")
                .body(aws_sdk_s3::primitives::ByteStream::from(captured.bytes.clone()))
                .send().await.unwrap();
        }
        let record = RevisionRecord {
            id: RevisionId::new(),
            document_id: DocumentId::new(),
            parents: vec![],
            operation: RevisionOperation::Content {
                path: relative.clone(),
                blob_hash: blob_hash.clone(),
            },
        };
        let metadata = serde_json::to_vec(&record).unwrap();
        writer.put_conditionally(
            &ns.immutable_revision_key(record.id), &metadata, S3PutCondition::IfAbsent,
        ).await.unwrap();
        let shard = S3ManifestShardV1::new(
            &ns,
            OperationId::new(),
            vec![S3RevisionRefV1 {
                revision_id: record.id,
                document_id: record.document_id,
                revision_blake3: blake3::hash(&metadata).to_hex().to_string(),
                blob_bytes: Some(bytes.len() as u64),
            }],
        ).unwrap();
        writer.store_manifest_object(shard.shard_id, &shard.encode(&ns).unwrap())
            .await.unwrap();
        let manifest = S3ManifestV1::new(
            &ns,
            OperationId::new(),
            1,
            None,
            vec![shard.reference(&ns).unwrap()],
        ).unwrap();
        writer.store_manifest_object(
            manifest.manifest_id, &manifest.encode(&ns).unwrap(),
        ).await.unwrap();
        let head = S3HeadPointerV1::new(
            &ns, 1, manifest.manifest_id, &manifest.digest(&ns).unwrap(), None,
        ).unwrap();
        let mut cloud_journal = S3HeadJournal::open(&app_a.join("s3-head.sqlite")).unwrap();
        assert_eq!(
            writer.publish_head_cas(&mut cloud_journal, manifest.manifest_id, head, None)
                .await.unwrap(),
            S3HeadPublication::Published
        );

        let verified = reader.materialize_verified_revision_metadata_store(256 * 1024)
            .await.unwrap().unwrap();
        let operation_id = OperationId::new();
        let intent = OperationIntent {
            operation_id,
            vault_id: vault,
            document_id: record.document_id,
            profile_id: "real-minio-test-profile".into(),
            revision_id: record.id,
            action: JournalAction::StageDownload,
            path: relative.clone(),
            remote_object_id: blob_key.clone(),
            remote_guard: None,
            expected_content_hash: blob_hash.clone(),
            expected_baseline: None,
        };
        let mut journal = SqliteSyncJournal::open(&app_b.join(crate::sync::RECOVERY_JOURNAL_FILENAME)).unwrap();
        let mut cache = SqliteDagStore::open(&app_b.join("blobs.sqlite")).unwrap();
        assert!(!root_b.join(relative.as_str()).exists());
        // An over-budget request must fail before the journal is changed,
        // regardless of any provider-reported Content-Length.
        assert!(matches!(
            reader.stage_verified_s3_content_for_core(
                &verified, &mut journal, &mut cache,
                intent.clone(), MAX_CORE_STAGE_BLOB_BYTES + 1,
            ).await,
            Err(S3CoreStageError::InvalidIntent)
        ));
        assert!(journal.get(operation_id).unwrap().is_none());
        if bytes.len() as u64 > super::super::MAX_PROBE_BYTES {
            // The small pointer/metadata transport MUST NOT become an
            // unbounded download just because the Core needs a large blob.
            assert!(matches!(
                reader.get_bounded(&blob_key, max_blob_bytes).await,
                Err(S3TransportError::ObjectTooLarge)
            ));
            assert!(matches!(
                reader.stage_verified_s3_content_for_core(
                    &verified, &mut journal, &mut cache,
                    intent.clone(), super::super::MAX_PROBE_BYTES,
                ).await,
                Err(S3CoreStageError::Transport(S3TransportError::ObjectTooLarge))
            ));
            assert!(journal.get(operation_id).unwrap().is_none());
        }
        let receipt = reader.stage_verified_s3_content_for_core(
            &verified, &mut journal, &mut cache, intent.clone(), max_blob_bytes,
        ).await.unwrap();
        assert_eq!(receipt.content_hash, blob_hash);
        assert_eq!(cache.verified_blob(&receipt.content_hash).unwrap(), bytes);
        assert!(!root_b.join(relative.as_str()).exists(), "S3 staging is not local apply");
        assert_eq!(journal.get(operation_id).unwrap().unwrap().phase, JournalPhase::Staged);
        assert!(journal.baseline(vault, record.document_id).unwrap().is_none());

        // A second independent Core must bind its LOCAL registration to the
        // verified SHARED S3 vault explicitly. A staged journal and BLAKE3
        // cache alone are not permission to bypass that durable association.
        let admission = RegisteredReadAdmission::open(&second_core, &root_b).unwrap();
        assert!(matches!(
            admission.create_staged_if_absent(operation_id, &journal, &cache),
            Err(crate::sync::host_admission::LocalCreateError::InvalidStagedIntent)
        ));
        let verified_head = reader.read_verified_head().await.unwrap().unwrap();
        let mut bindings = SqliteRegisteredS3Bindings::open(
            &admission, &app_b.join("s3-profiles.sqlite"),
        ).unwrap();
        let bound = bindings.bind_existing_verified_s3(
            &admission, "real-minio-test-profile", &profile, &ns, &verified_head,
        ).unwrap();
        assert_eq!(bound.workspace_id, target_id);
        assert_eq!(bound.vault_id, vault);

        // A verified S3 head/profile alone is not an OS write interlock.
        // Provision one EXTERNAL, pre-existing lock fixture and separately
        // persist its exact OS identity before any File Gate publication.
        let custody_scope = dir.path().join("host-owned-custody")
            .join(scoped_leaf(PinnedProbeRoot::open(&root_b).unwrap().identity()));
        fs::create_dir_all(&custody_scope).unwrap();
        let custody_file = custody_scope.join(LOCK_NAME);
        fs::write(&custody_file, b"pre-existing external lock").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&custody_scope, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&custody_file, fs::Permissions::from_mode(0o600)).unwrap();
        }
        #[cfg(unix)]
        let custody_scope = fs::canonicalize(custody_scope).unwrap();
        let mut custody = SqliteCustodyIdentityStore::open(
            &admission, &app_b.join("s3-custody.sqlite"),
        ).unwrap();
        custody.bind_registered_s3(&admission, &bindings, &custody_scope).unwrap();

        if receiver_in_child {
            // The parent only staged the object and provisioned independently
            // checked host/profile custody. It now drops EVERY receiver Core
            // and SQLite handle before launching a fresh OS process to
            // re-fetch the verified S3 head and perform guarded publication.
            drop(custody);
            drop(bindings);
            drop(admission);
            drop(cache);
            drop(journal);
            drop(second_core);
            const CHILD: &str =
                "sync::s3_transport::core_stage::tests::s3_e2e01_receiver_child_entrypoint";
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--exact").arg(CHILD).arg("--nocapture")
                .env("AKULA_S3_E2E01_CHILD_INTENT",
                     serde_json::to_string(&intent).unwrap())
                .env("AKULA_S3_E2E01_ROOT", &root_b)
                .env("AKULA_S3_E2E01_APPDATA", &app_b)
                .env("AKULA_S3_E2E01_CUSTODY_SCOPE", &custody_scope)
                .env("AKULA_S3_E2E01_LIMIT", max_blob_bytes.to_string())
                .env("AQ_S3_TEST_ENDPOINT", profile.endpoint.as_deref().unwrap())
                .output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(),
                "second Akula process failed: {stdout}\n{stderr}");
            assert!(stdout.contains(&format!("test {CHILD} ...")),
                "exact child libtest did not execute: {stdout}");
            assert!(stdout.contains("test result: ok. 1 passed; 0 failed;"),
                "child libtest did not pass: {stdout}");
            assert!(stdout.contains("AKULA_S3_E2E01_CHILD=UNCONFIRMED_LOCAL_CREATE"),
                "child did not report verified but unconfirmed local creation: {stdout}");

            // A third independent SQLite opening (parent after child exit)
            // must see only Staged and no synthetic confirmed baseline.
            let cold_journal = SqliteSyncJournal::open(&app_b.join(crate::sync::RECOVERY_JOURNAL_FILENAME)).unwrap();
            assert_eq!(cold_journal.get(operation_id).unwrap().unwrap().phase,
                JournalPhase::Staged);
            assert!(cold_journal.baseline(vault, record.document_id).unwrap().is_none());
            assert_eq!(cold_journal.local_publication_review_queue().unwrap().len(), 1);
            assert_eq!(fs::read(root_b.join(relative.as_str())).unwrap(), bytes);
            let cold_core = Core::open(&app_b, Arc::new(|_: CoreEvent| {}));
            match crate::sync::native_s3_bound_publication_overview(
                &cold_core, &app_b, &root_b,
            ) {
                crate::sync::NativeRecoveryOverview::RequiresReview {
                    total_claims, examples,
                } => {
                    assert_eq!(total_claims, 1);
                    assert_eq!(examples[0].operation_id, operation_id.to_string());
                    assert_eq!(examples[0].historical_path, relative.as_str());
                }
                other => panic!("cold Native S3 recovery lost the claim: {other:?}"),
            }
            assert!(cold_journal.baseline(vault, record.document_id).unwrap().is_none());
            return;
        }

        // Exercise the full verified-head -> cache -> host File Gate path.
        let published = reader.receive_single_verified_s3_additive_once(
            &verified, &admission, &bindings, &custody, &custody_scope,
            &mut journal, &mut cache, intent.clone(), max_blob_bytes,
        ).await.unwrap();
        assert_eq!(published.byte_length, bytes.len() as u64);
        assert_eq!(fs::read(root_b.join(relative.as_str())).unwrap(), bytes);
        assert!(matches!(
            admission.create_staged_if_absent_for_registered_s3(
                operation_id, &journal, &cache, &bindings, &profile, &ns,
                &custody, &custody_scope,
            ),
            Err(crate::sync::host_admission::LocalCreateError::Admission(
                VaultPortError::DestinationExists
            ))
        ));
        assert_eq!(fs::read(root_b.join(relative.as_str())).unwrap(), bytes);
        assert!(matches!(
            journal.mark_applied(operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
        assert_eq!(journal.get(operation_id).unwrap().unwrap().phase, JournalPhase::Staged);
        assert!(journal.baseline(vault, record.document_id).unwrap().is_none());
        assert_eq!(journal.local_publication_review_queue().unwrap().len(), 1);

        // Replay after a journal/cache restart must not create another file
        // or grant a baseline. The cloud bytes remain independently readable.
        drop(admission);
        drop(cache);
        drop(journal);
        let journal = SqliteSyncJournal::open(&app_b.join(crate::sync::RECOVERY_JOURNAL_FILENAME)).unwrap();
        let cache = SqliteDagStore::open(&app_b.join("blobs.sqlite")).unwrap();
        assert!(RegisteredReadAdmission::open(&second_core, &root_b).unwrap()
            .create_staged_if_absent_for_registered_s3(
                operation_id, &journal, &cache, &bindings, &profile, &ns,
                &custody, &custody_scope,
            ).is_err());
        assert!(journal.baseline(vault, record.document_id).unwrap().is_none());
        assert_eq!(fs::read(root_b.join(relative.as_str())).unwrap(), bytes);
    }
