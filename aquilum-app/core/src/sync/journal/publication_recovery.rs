use super::*;

impl SqliteSyncJournal {
    /// Claim a STAGED local-create attempt exactly once, durably BEFORE any
    /// filesystem stage/create/rename can begin. SQLite WAL + FULL and a
    /// single conditional INSERT provide a cross-process, no-replay fence.
    /// An earlier attempt on this UUID OR on the same portable vault path
    /// fails closed, even if its process died before touching the filesystem.
    ///
    /// Do not turn a successful insert into a "preserved" or baseline receipt.
    pub fn record_local_publication_attempt_once(
        &self,
        intent: &OperationIntent,
    ) -> JournalResult<()> {
        let existing = self
            .get(intent.operation_id)?
            .ok_or(JournalError::MissingOperation)?;
        if existing.intent != *intent {
            return Err(JournalError::IntentCollision);
        }
        if existing.phase != JournalPhase::Staged
            || existing.staged_hash.as_deref() != Some(intent.expected_content_hash.as_str())
            || intent.action != JournalAction::StageDownload
            || intent.remote_guard.is_some()
            || intent.expected_baseline.is_some()
            || self
                .baseline(intent.vault_id, intent.document_id)?
                .is_some()
        {
            return Err(JournalError::InvalidTransition);
        }
        let digest = blake3::hash(&serde_json::to_vec(intent)?)
            .to_hex()
            .to_string();
        // The WHERE clause revalidates phase, digest and staged hash at the
        // moment of durable publication admission, not only in an earlier GET.
        let changed = self.connection.execute(
            "INSERT OR IGNORE INTO sync_local_publication_attempts
             (operation_id,vault_id,path_collision_key,relative_path,expected_hash,intent_digest)
             SELECT ?1,?2,?3,?4,?5,?6 FROM sync_operations
             WHERE operation_id = ?1 AND phase = 1
               AND staged_hash = ?5 AND digest = ?6",
            params![
                intent.operation_id.to_string(),
                intent.vault_id.to_string(),
                intent.path.collision_key(),
                intent.path.as_str(),
                &intent.expected_content_hash,
                digest,
            ],
        )?;
        if changed != 1 {
            // Could be another worker, a previous crash, a name collision,
            // or a raced phase change. None justifies a retry.
            return Err(JournalError::PublicationAttemptAlreadyRecorded);
        }
        if !self.local_publication_review_queue()?.iter().any(|entry| {
            entry.operation_id == intent.operation_id
                && entry.vault_id == intent.vault_id
                && entry.path == intent.path
                && entry.expected_content_hash == intent.expected_content_hash
        }) {
            return Err(JournalError::CorruptedRecord);
        }
        Ok(())
    }

    /// Durable once-only REPORT after native Unix publication, never a safe
    /// replay token. Caller must keep the Core File Gate and already have
    /// checked the OS file/parent IDs plus successful parent-directory fsync.
    /// An interrupted process prior to the SQLite transaction remains HOLD.
    #[cfg(unix)]
    pub(crate) fn record_native_local_publication_observation_once(
        &self,
        obs: &NativeLocalPublicationObservation,
    ) -> JournalResult<()> {
        if obs.parent.device == 0
            || obs.parent.file == 0
            || obs.final_file.device == 0
            || obs.final_file.file == 0
            || obs.byte_length > 32 * 1024 * 1024
            || obs.expected_content_hash.len() != 64
        {
            return Err(JournalError::InvalidTransition);
        }
        let record = self
            .get(obs.operation_id)?
            .ok_or(JournalError::MissingOperation)?;
        if record.phase != JournalPhase::Staged
            || record.intent.action != JournalAction::StageDownload
            || record.intent.remote_guard.is_some()
            || record.intent.expected_baseline.is_some()
            || record.intent.vault_id != obs.vault_id
            || record.intent.path != obs.path
            || record.intent.expected_content_hash != obs.expected_content_hash
            || record.staged_hash.as_deref() != Some(obs.expected_content_hash.as_str())
            || self
                .baseline(obs.vault_id, record.intent.document_id)?
                .is_some()
        {
            return Err(JournalError::InvalidTransition);
        }
        let claim = self
            .local_publication_review_queue()?
            .into_iter()
            .find(|row| row.operation_id == obs.operation_id)
            .ok_or(JournalError::InvalidTransition)?;
        if claim.vault_id != obs.vault_id
            || claim.path != obs.path
            || claim.expected_content_hash != obs.expected_content_hash
        {
            return Err(JournalError::CorruptedRecord);
        }
        let payload = serde_json::to_vec(obs)?;
        let digest = blake3::hash(&payload).to_hex().to_string();
        let n = self.connection.execute(
            "INSERT OR IGNORE INTO sync_local_native_publication_observations
             (operation_id,observation_json,observation_digest)
             SELECT ?1,?2,?3 FROM sync_operations AS op
             JOIN sync_local_publication_attempts AS claim
               ON op.operation_id = claim.operation_id
             WHERE op.operation_id = ?1 AND op.phase = 1
               AND op.staged_hash = ?4 AND claim.expected_hash = ?4
               AND claim.vault_id = ?5 AND claim.relative_path = ?6",
            params![
                obs.operation_id.to_string(),
                payload,
                digest,
                &obs.expected_content_hash,
                obs.vault_id.to_string(),
                obs.path.as_str()
            ],
        )?;
        if n != 1 {
            return Err(JournalError::NativePublicationObservationAlreadyRecorded);
        }
        if self
            .read_native_local_publication_observation(obs.operation_id)?
            .as_ref()
            != Some(obs)
        {
            return Err(JournalError::CorruptedRecord);
        }
        Ok(())
    }

    /// Informational, corruption-checked read; even a verified observation
    /// cannot authorize local mutation, lost-ACK replay, or sync baseline.
    pub(crate) fn read_native_local_publication_observation(
        &self,
        operation: OperationId,
    ) -> JournalResult<Option<NativeLocalPublicationObservation>> {
        let row: Option<(Vec<u8>, String)> = self
            .connection
            .query_row(
                "SELECT observation_json,observation_digest
             FROM sync_local_native_publication_observations WHERE operation_id = ?1",
                [operation.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((bytes, digest)) = row else {
            return Ok(None);
        };
        if digest != blake3::hash(&bytes).to_hex().as_str() {
            return Err(JournalError::CorruptedRecord);
        }
        let obs: NativeLocalPublicationObservation =
            serde_json::from_slice(&bytes).map_err(|_| JournalError::CorruptedRecord)?;
        let rec = self.get(operation)?.ok_or(JournalError::CorruptedRecord)?;
        if obs.operation_id != operation
            || rec.phase != JournalPhase::Staged
            || rec.intent.action != JournalAction::StageDownload
            || rec.intent.vault_id != obs.vault_id
            || rec.intent.path != obs.path
            || rec.intent.expected_content_hash != obs.expected_content_hash
            || rec.staged_hash.as_deref() != Some(obs.expected_content_hash.as_str())
            || obs.parent.device == 0
            || obs.parent.file == 0
            || obs.final_file.device == 0
            || obs.final_file.file == 0
            || obs.byte_length > 32 * 1024 * 1024
        {
            return Err(JournalError::CorruptedRecord);
        }
        let claim = self
            .local_publication_review_queue()?
            .into_iter()
            .find(|row| row.operation_id == operation)
            .ok_or(JournalError::CorruptedRecord)?;
        if claim.vault_id != obs.vault_id
            || claim.path != obs.path
            || claim.expected_content_hash != obs.expected_content_hash
        {
            return Err(JournalError::CorruptedRecord);
        }
        Ok(Some(obs))
    }

    /// Persisted recovery/quarantine *queue*, not a cleanup executor.
    /// Treat every returned entry as requiring human/host investigation.
    /// In particular, a missing stage or a same-hash destination is NOT
    /// proof a lost acknowledgment was safe to replay or commit.
    /// Cheap, bounded-dashboard preflight: decline to enumerate an enormous
    /// or unexpectedly populated journal in a synchronous Settings render.
    pub(crate) fn local_publication_count(&self) -> JournalResult<u64> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM sync_local_publication_attempts",
            [],
            |row| row.get(0),
        )?;
        u64::try_from(count).map_err(|_| JournalError::CorruptedRecord)
    }

    pub fn local_publication_review_queue(&self) -> JournalResult<Vec<LocalPublicationReview>> {
        let mut stmt = self.connection.prepare(
            "SELECT operation_id, vault_id, path_collision_key, relative_path,
                    expected_hash, intent_digest
             FROM sync_local_publication_attempts
             ORDER BY vault_id, path_collision_key, operation_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (raw_id, raw_vault, key, raw_path, hash, digest) = row?;
            let id: OperationId = raw_id.parse().map_err(|_| JournalError::CorruptedRecord)?;
            let stored_vault: VaultId = raw_vault
                .parse()
                .map_err(|_| JournalError::CorruptedRecord)?;
            let record = self.get(id)?.ok_or(JournalError::CorruptedRecord)?;
            let parsed_path =
                SyncRelativePath::parse(&raw_path).map_err(|_| JournalError::CorruptedRecord)?;
            if record.intent.vault_id != stored_vault
                || record.intent.path != parsed_path
                || record.intent.path.as_str() != raw_path
                || record.intent.path.collision_key() != key
                || record.intent.expected_content_hash != hash
                || blake3::hash(&serde_json::to_vec(&record.intent)?)
                    .to_hex()
                    .as_str()
                    != digest
                || record.intent.action != JournalAction::StageDownload
                || record.phase != JournalPhase::Staged
            {
                return Err(JournalError::CorruptedRecord);
            }
            result.push(LocalPublicationReview {
                operation_id: id,
                vault_id: stored_vault,
                path: parsed_path,
                expected_content_hash: hash,
            });
        }
        Ok(result)
    }

    /// Compare a proposed non-authoritative incident snapshot with the
    /// immutable STAGED publication claim. No user file or Sync baseline is
    /// modified. This does not certify who created a matching file.
    fn check_local_recovery_case(&self, case: &LocalRecoveryEvidenceCase) -> JournalResult<()> {
        let claim = self
            .local_publication_review_queue()?
            .into_iter()
            .find(|entry| entry.operation_id == case.operation_id)
            .ok_or(JournalError::CorruptedRecord)?;
        if case.vault_id != claim.vault_id
            || case.relative_path != claim.path
            || case.expected_content_hash != claim.expected_content_hash
            || case.evidence.operation_id != case.operation_id
            || case.evidence.assessment
                != local_recovery::classify(&case.evidence.stage, &case.evidence.final_name)
            || case.evidence.parent_identity.file == 0
        {
            return Err(JournalError::CorruptedRecord);
        }
        // This cap also protects deserialized, tampered evidence records
        // from being represented as verified, bounded local observations.
        for observed in [&case.evidence.stage, &case.evidence.final_name] {
            if let ObservedLocalFile::Present {
                identity,
                byte_length,
                ..
            } = observed
            {
                if identity.file == 0 || *byte_length > 32 * 1024 * 1024 {
                    return Err(JournalError::CorruptedRecord);
                }
            }
        }
        Ok(())
    }

    /// Make the FIRST observed recovery state durable in the *existing*
    /// app-data WAL/FULL SQLite journal. A later same-byte/different-OS-ID
    /// observation cannot overwrite it. No cleanup, replay or CAS receipt.
    pub(crate) fn record_local_recovery_case_once(
        &self,
        case: &LocalRecoveryEvidenceCase,
    ) -> JournalResult<()> {
        self.check_local_recovery_case(case)?;
        let payload = serde_json::to_vec(case)?;
        let digest = blake3::hash(&payload).to_hex().to_string();
        let count = self.connection.execute(
            "INSERT OR IGNORE INTO sync_local_recovery_evidence
                 (operation_id, evidence_json, evidence_digest)
             SELECT ?1, ?2, ?3 FROM sync_local_publication_attempts AS claim
             JOIN sync_operations AS operation
               ON claim.operation_id = operation.operation_id
             WHERE claim.operation_id = ?1 AND operation.phase = 1",
            params![case.operation_id.to_string(), &payload, &digest],
        )?;
        if count != 1 {
            return Err(JournalError::RecoveryEvidenceAlreadyRecorded);
        }
        let loaded = self
            .read_local_recovery_case(case.operation_id)?
            .ok_or(JournalError::CorruptedRecord)?;
        if loaded != *case {
            return Err(JournalError::CorruptedRecord);
        }
        Ok(())
    }

    /// Read back exact durable audit evidence. It remains a historical
    /// observation, not evidence that the filesystem still looks this way.
    pub(crate) fn read_local_recovery_case(
        &self,
        operation: OperationId,
    ) -> JournalResult<Option<LocalRecoveryEvidenceCase>> {
        let row: Option<(Vec<u8>, String)> = self
            .connection
            .query_row(
                "SELECT evidence_json, evidence_digest
             FROM sync_local_recovery_evidence WHERE operation_id = ?1",
                [operation.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((payload, expected_digest)) = row else {
            return Ok(None);
        };
        if expected_digest != blake3::hash(&payload).to_hex().as_str() {
            return Err(JournalError::CorruptedRecord);
        }
        let case: LocalRecoveryEvidenceCase =
            serde_json::from_slice(&payload).map_err(|_| JournalError::CorruptedRecord)?;
        if case.operation_id != operation {
            return Err(JournalError::CorruptedRecord);
        }
        self.check_local_recovery_case(&case)?;
        Ok(Some(case))
    }

    /// Refuse to persist evidence bytes in an arbitrary database that might
    /// live under the registered user's sync root. The caller still needs a
    /// trusted, private app-data directory (including all SQLite sidecars).
    /// Canonical path checks complement, but do not replace, host authority.
    pub(crate) fn require_appdata_outside_vault(&self, root: &Path) -> JournalResult<()> {
        let mut statement = self.connection.prepare("PRAGMA database_list")?;
        let mut rows = statement.query([])?;
        let mut path = None;
        while let Some(row) = rows.next()? {
            if row.get::<_, String>(1)? == "main" {
                path = Some(row.get::<_, String>(2)?);
                break;
            }
        }
        let raw = path.ok_or(JournalError::UnsafeArchiveLocation)?;
        if raw.is_empty() {
            return Err(JournalError::UnsafeArchiveLocation);
        }
        let file = std::fs::canonicalize(raw).map_err(|_| JournalError::UnsafeArchiveLocation)?;
        let root = std::fs::canonicalize(root).map_err(|_| JournalError::UnsafeArchiveLocation)?;
        if file.starts_with(&root) {
            return Err(JournalError::UnsafeArchiveLocation);
        }
        let metadata = std::fs::metadata(&file).map_err(|_| JournalError::UnsafeArchiveLocation)?;
        if !metadata.is_file() {
            return Err(JournalError::UnsafeArchiveLocation);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1 {
                return Err(JournalError::UnsafeArchiveLocation);
            }
        }
        Ok(())
    }

    fn validated_forensic_digest(
        observed: &ObservedLocalFile,
        source: Option<&[u8]>,
        expected_hash: &str,
    ) -> JournalResult<Option<String>> {
        match (observed, source) {
            (ObservedLocalFile::Absent, None) => Ok(None),
            (
                ObservedLocalFile::Present {
                    byte_length,
                    matches_expected_hash,
                    ..
                },
                Some(bytes),
            ) if *byte_length <= 32 * 1024 * 1024 && *byte_length == bytes.len() as u64 => {
                let digest = blake3::hash(bytes).to_hex().to_string();
                if (digest == expected_hash) != *matches_expected_hash {
                    return Err(JournalError::CorruptedRecord);
                }
                Ok(Some(digest))
            }
            _ => Err(JournalError::CorruptedRecord),
        }
    }

    /// Copy bytes into existing app-data SQLite WAL/FULL without changing
    /// the vault. The first complete copy is permanent and integrity-checked.
    /// A known *different* final file can also be copied for human review,
    /// but may never be installed or deleted based on this archive alone.
    pub(crate) fn record_local_recovery_copy_once(
        &self,
        operation: OperationId,
        stage_bytes: Option<&[u8]>,
        final_bytes: Option<&[u8]>,
    ) -> JournalResult<LocalRecoveryArchivedCopy> {
        let case = self
            .read_local_recovery_case(operation)?
            .ok_or(JournalError::CorruptedRecord)?;
        let stage_hash = Self::validated_forensic_digest(
            &case.evidence.stage,
            stage_bytes,
            &case.expected_content_hash,
        )?;
        let final_hash = Self::validated_forensic_digest(
            &case.evidence.final_name,
            final_bytes,
            &case.expected_content_hash,
        )?;
        let changed = self.connection.execute(
            "INSERT OR IGNORE INTO sync_local_recovery_copies
                 (operation_id, stage_bytes, stage_hash, final_bytes, final_hash)
             SELECT ?1, ?2, ?3, ?4, ?5
             FROM sync_local_recovery_evidence WHERE operation_id = ?1",
            params![
                operation.to_string(),
                stage_bytes,
                stage_hash,
                final_bytes,
                final_hash,
            ],
        )?;
        if changed != 1 {
            return Err(JournalError::RecoveryCopyAlreadyRecorded);
        }
        let loaded = self
            .read_local_recovery_copy(operation)?
            .ok_or(JournalError::CorruptedRecord)?;
        if loaded.stage_bytes.as_deref() != stage_bytes
            || loaded.final_bytes.as_deref() != final_bytes
        {
            return Err(JournalError::CorruptedRecord);
        }
        Ok(loaded)
    }

    /// Return durable copied bytes only if their contents and the immutable
    /// recorded file identities / classifications remain internally consistent.
    /// This is *historical forensic data*, not a replay or cleanup authority.
    pub(crate) fn read_local_recovery_copy(
        &self,
        operation: OperationId,
    ) -> JournalResult<Option<LocalRecoveryArchivedCopy>> {
        let row: Option<(
            Option<Vec<u8>>,
            Option<String>,
            Option<Vec<u8>>,
            Option<String>,
        )> = self
            .connection
            .query_row(
                "SELECT stage_bytes, stage_hash, final_bytes, final_hash
                 FROM sync_local_recovery_copies WHERE operation_id = ?1",
                [operation.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((stage, stage_hash, final_name, final_hash)) = row else {
            return Ok(None);
        };
        let case = self
            .read_local_recovery_case(operation)?
            .ok_or(JournalError::CorruptedRecord)?;
        let observed_stage = Self::validated_forensic_digest(
            &case.evidence.stage,
            stage.as_deref(),
            &case.expected_content_hash,
        )?;
        let observed_final = Self::validated_forensic_digest(
            &case.evidence.final_name,
            final_name.as_deref(),
            &case.expected_content_hash,
        )?;
        if stage_hash != observed_stage || final_hash != observed_final {
            return Err(JournalError::CorruptedRecord);
        }
        Ok(Some(LocalRecoveryArchivedCopy {
            operation_id: operation,
            stage_bytes: stage,
            final_bytes: final_name,
        }))
    }

    /// A read-only, bounded snapshot for a future host-native human review
    /// screen. No original bytes are exposed by this API; the opaque
    /// fingerprint is bound to the first immutable incident + forensic copy.
    /// Retention is ALWAYS indefinite until a separately approved purge gate.
    pub(crate) fn recovery_review_snapshot(
        &self,
        operation: OperationId,
    ) -> JournalResult<LocalRecoveryReviewSnapshot> {
        let incident = self
            .read_local_recovery_case(operation)?
            .ok_or(JournalError::MissingRecoveryArchive)?;
        let copy = self
            .read_local_recovery_copy(operation)?
            .ok_or(JournalError::MissingRecoveryArchive)?;
        let fingerprint = recovery_review::archive_fingerprint(&incident, &copy)?;
        let history = Self::read_recovery_review_rows(&self.connection, operation, &fingerprint)?;
        let latest_event_digest = match history.last() {
            Some(event) => blake3::hash(&serde_json::to_vec(event)?)
                .to_hex()
                .to_string(),
            None => "genesis".to_string(),
        };
        Ok(LocalRecoveryReviewSnapshot {
            operation_id: operation,
            vault_id: incident.vault_id,
            path: incident.relative_path,
            assessment: incident.evidence.assessment,
            archive_fingerprint: fingerprint,
            review_count: history.len() as u32,
            latest_event_digest,
            retention: RECOVERY_REVIEW_RETENTION,
        })
    }

    /// Every review event is append-only, chained by BLAKE3 to the previous
    /// event, and checked against the *same* historical immutable archive.
    /// A hash chain detects accidental data damage, not privileged SQLite
    /// tampering by an attacker who can rewrite the database and all digests.
    fn read_recovery_review_rows(
        connection: &Connection,
        operation: OperationId,
        archive_fingerprint: &str,
    ) -> JournalResult<Vec<LocalRecoveryReviewEvent>> {
        let mut statement = connection.prepare(
            "SELECT sequence, review_json, review_digest
             FROM sync_local_recovery_reviews
             WHERE operation_id = ?1 ORDER BY sequence",
        )?;
        let mut rows = statement.query([operation.to_string()])?;
        let mut events = Vec::new();
        let mut previous_digest = "genesis".to_string();
        while let Some(row) = rows.next()? {
            let sequence: i64 = row.get(0)?;
            let payload: Vec<u8> = row.get(1)?;
            let expected_digest: String = row.get(2)?;
            let actual_digest = blake3::hash(&payload).to_hex().to_string();
            let event: LocalRecoveryReviewEvent =
                serde_json::from_slice(&payload).map_err(|_| JournalError::CorruptedRecord)?;
            if sequence != (events.len() + 1) as i64
                || sequence > 128
                || event.sequence as i64 != sequence
                || event.operation_id != operation
                || event.previous_event_digest != previous_digest
                || event.archive_fingerprint != archive_fingerprint
                || event.retention != RECOVERY_REVIEW_RETENTION
                || recovery_review::validated_review_note(&event.note).as_deref()
                    != Some(event.note.as_str())
                || expected_digest != actual_digest
            {
                return Err(JournalError::CorruptedRecord);
            }
            previous_digest = actual_digest;
            events.push(event);
        }
        Ok(events)
    }

    pub(crate) fn recovery_review_history(
        &self,
        operation: OperationId,
    ) -> JournalResult<Vec<LocalRecoveryReviewEvent>> {
        let snapshot = self.recovery_review_snapshot(operation)?;
        Self::read_recovery_review_rows(&self.connection, operation, &snapshot.archive_fingerprint)
    }

    /// Metadata-only future operator intent; no physical action is attached
    /// to either allowed disposition. Caller must supply both the previously
    /// displayed archive fingerprint and the latest event revision/digest.
    /// For safety, a stale screen or concurrent editor cannot silently append.
    pub(crate) fn append_recovery_review(
        &mut self,
        operation: OperationId,
        expected_vault: VaultId,
        expected_path: &SyncRelativePath,
        expected_archive_fingerprint: &str,
        expected_review_count: u32,
        expected_previous_digest: &str,
        disposition: RecoveryReviewDisposition,
        note: &str,
    ) -> JournalResult<LocalRecoveryReviewEvent> {
        let note = recovery_review::validated_review_note(note)
            .ok_or(JournalError::InvalidRecoveryReview)?;
        let snapshot = self.recovery_review_snapshot(operation)?;
        if snapshot.vault_id != expected_vault
            || &snapshot.path != expected_path
            || snapshot.archive_fingerprint != expected_archive_fingerprint
            || snapshot.review_count != expected_review_count
            || snapshot.latest_event_digest != expected_previous_digest
        {
            return Err(JournalError::StaleRecoveryReview);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous =
            Self::read_recovery_review_rows(&tx, operation, &snapshot.archive_fingerprint)?;
        let last_digest = match previous.last() {
            Some(event) => blake3::hash(&serde_json::to_vec(event)?)
                .to_hex()
                .to_string(),
            None => "genesis".to_string(),
        };
        if previous.len() as u32 != expected_review_count
            || last_digest != expected_previous_digest
            || previous.len() >= 128
        {
            return Err(JournalError::StaleRecoveryReview);
        }
        let event = LocalRecoveryReviewEvent {
            operation_id: operation,
            sequence: previous.len() as u32 + 1,
            archive_fingerprint: snapshot.archive_fingerprint,
            disposition,
            note,
            previous_event_digest: last_digest,
            retention: RECOVERY_REVIEW_RETENTION.to_string(),
        };
        let bytes = serde_json::to_vec(&event)?;
        let digest = blake3::hash(&bytes).to_hex().to_string();
        tx.execute(
            "INSERT INTO sync_local_recovery_reviews
                 (operation_id, sequence, review_json, review_digest)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                operation.to_string(),
                i64::from(event.sequence),
                &bytes,
                digest,
            ],
        )?;
        tx.commit()?;
        Ok(event)
    }
}
