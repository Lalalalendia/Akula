use super::*;

impl SqliteSyncJournal {
    /// Same operation ID and intent is idempotent; reusing the UUID for a
    /// different action is rejected. One in-flight operation per document
    /// prevents two workers from publishing incompatible baselines.
    pub fn begin(&mut self, intent: OperationIntent) -> JournalResult<JournalPhase> {
        intent.validate()?;
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = read_operation(&transaction, intent.operation_id)? {
            if existing.intent != intent {
                return Err(JournalError::IntentCollision);
            }
            return Ok(existing.phase);
        }
        let baseline = load_baseline(&transaction, intent.vault_id, intent.document_id)?;
        if baseline.as_ref().is_some_and(|b| b.profile_id != intent.profile_id) {
            return Err(JournalError::ProfileChanged);
        }
        if baseline.map(|b| b.revision_id) != intent.expected_baseline {
            return Err(JournalError::BaselineChanged);
        }

        let conflicting: Option<String> = transaction.query_row(
            "SELECT operation_id FROM sync_operations
             WHERE vault_id = ?1 AND document_id = ?2 AND phase < 4 LIMIT 1",
            params![intent.vault_id.to_string(), intent.document_id.to_string()],
            |row| row.get(0),
        ).optional()?;
        if conflicting.is_some() {
            return Err(JournalError::BusyDocument);
        }
        let payload = serde_json::to_vec(&intent)?;
        let digest = blake3::hash(&payload).to_hex().to_string();
        transaction.execute(
            "INSERT INTO sync_operations
             (operation_id,vault_id,document_id,intent,digest,phase)
             VALUES (?1,?2,?3,?4,?5,0)",
            params![
                intent.operation_id.to_string(),
                intent.vault_id.to_string(),
                intent.document_id.to_string(), payload, digest,
            ],
        )?;
        transaction.commit()?;
        Ok(JournalPhase::Pending)
    }

    /// \`staged_bytes\` must come from a verified immutable blob or a completed
    /// remote download. Computing the expected hash locally prevents recording
    /// a fake "stage complete" receipt for corrupted contents.
    /// A stage is metadata only: PR 04 must still obtain a protective copy
    /// before a destructive local File Gate operation.
    pub fn mark_staged(&mut self, id: OperationId, staged_bytes: &[u8])
        -> JournalResult<JournalPhase>
    {
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = read_operation(&transaction, id)?.ok_or(JournalError::MissingOperation)?;
        let digest = blake3::hash(staged_bytes).to_hex().to_string();
        if digest != record.intent.expected_content_hash {
            return Err(JournalError::ReadbackMismatch);
        }
        match record.phase {
            JournalPhase::Pending => {
                transaction.execute(
                    "UPDATE sync_operations SET phase = 1, staged_hash = ?2
                     WHERE operation_id = ?1 AND phase = 0",
                    params![id.to_string(), &digest],
                )?;
                transaction.commit()?;
                Ok(JournalPhase::Staged)
            }
            JournalPhase::Staged if record.staged_hash.as_deref() == Some(digest.as_str()) =>
                Ok(JournalPhase::Staged),
            _ => Err(JournalError::InvalidTransition),
        }
    }

    /// The caller must only invoke this after the IO executor has attempted
    /// the real provider/File Gate mutation. An uncertain response is NOT a
    /// receipt: recovery from Staged also demands inspection, never blind redo.
    pub fn mark_applied(&mut self, id: OperationId) -> JournalResult<JournalPhase> {
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = read_operation(&transaction, id)?.ok_or(JournalError::MissingOperation)?;
        // A remote download has no native, crash-durable local apply receipt.
        // Generic provider readback/hash must never turn it into a baseline.
        if record.intent.action == JournalAction::StageDownload {
            return Err(JournalError::LocalDownloadNeedsNativeReceipt);
        }
        match record.phase {
            JournalPhase::Staged if record.staged_hash.as_deref()
                == Some(record.intent.expected_content_hash.as_str()) => {
                transaction.execute(
                    "UPDATE sync_operations SET phase = 2
                     WHERE operation_id = ?1 AND phase = 1",
                    [id.to_string()],
                )?;
                transaction.commit()?;
                Ok(JournalPhase::Applied)
            }
            JournalPhase::Applied => Ok(JournalPhase::Applied),
            _ => Err(JournalError::InvalidTransition),
        }
    }

    /// Independently read-back bytes and the current opaque provider validator.
    /// The caller is responsible for fetching *actual destination bytes*;
    /// passing staged source bytes instead would violate the executor contract.
    pub fn mark_verified(&mut self, id: OperationId, observed_bytes: &[u8],
        provider_revision: &str) -> JournalResult<JournalPhase>
    {
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = read_operation(&transaction, id)?.ok_or(JournalError::MissingOperation)?;
        // A remote download has no native, crash-durable local apply receipt.
        // Generic provider readback/hash must never turn it into a baseline.
        if record.intent.action == JournalAction::StageDownload {
            return Err(JournalError::LocalDownloadNeedsNativeReceipt);
        }
        let digest = blake3::hash(observed_bytes).to_hex().to_string();
        if digest != record.intent.expected_content_hash || provider_revision.is_empty() {
            return Err(JournalError::ReadbackMismatch);
        }
        match record.phase {
            JournalPhase::Applied => {
                transaction.execute(
                    "UPDATE sync_operations SET phase = 3, readback_hash = ?2,
                     provider_revision = ?3 WHERE operation_id = ?1 AND phase = 2",
                    params![id.to_string(), &digest, provider_revision],
                )?;
                transaction.commit()?;
                Ok(JournalPhase::Verified)
            }
            JournalPhase::Verified if record.readback_hash.as_deref() == Some(digest.as_str())
                && record.provider_revision.as_deref() == Some(provider_revision) =>
                Ok(JournalPhase::Verified),
            _ => Err(JournalError::InvalidTransition),
        }
    }

    /// Commit baseline and terminal journal state IN ONE transaction. Recheck
    /// the expected previous baseline even if validation happened at begin().
    /// A failed SQLite transaction leaves the operation Verified and retryable.
    pub fn commit(&mut self, id: OperationId) -> JournalResult<JournalPhase> {
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = read_operation(&transaction, id)?.ok_or(JournalError::MissingOperation)?;
        // A remote download has no native, crash-durable local apply receipt.
        // Generic provider readback/hash must never turn it into a baseline.
        if record.intent.action == JournalAction::StageDownload {
            return Err(JournalError::LocalDownloadNeedsNativeReceipt);
        }
        if record.phase == JournalPhase::Committed {
            return Ok(JournalPhase::Committed);
        }
        if record.phase != JournalPhase::Verified
            || record.readback_hash.as_deref()
                != Some(record.intent.expected_content_hash.as_str())
            || record.provider_revision.as_deref().is_none_or(str::is_empty)
        {
            return Err(JournalError::InvalidTransition);
        }
        let intent = &record.intent;
        let baseline = load_baseline(&transaction, intent.vault_id, intent.document_id)?;
        if baseline.as_ref().is_some_and(|b| b.profile_id != intent.profile_id) {
            return Err(JournalError::ProfileChanged);
        }
        if baseline.map(|b| b.revision_id) != intent.expected_baseline {
            return Err(JournalError::BaselineChanged);
        }
        transaction.execute(
            "INSERT INTO sync_baselines (vault_id,document_id,revision_id,content_hash,operation_id)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(vault_id,document_id) DO UPDATE SET
                 revision_id = excluded.revision_id,
                 content_hash = excluded.content_hash,
                 operation_id = excluded.operation_id",
            params![
                intent.vault_id.to_string(),
                intent.document_id.to_string(),
                intent.revision_id.to_string(),
                &intent.expected_content_hash,
                id.to_string(),
            ],
        )?;
        transaction.execute(
            "UPDATE sync_operations SET phase = 4
             WHERE operation_id = ?1 AND phase = 3",
            [id.to_string()],
        )?;
        transaction.commit()?;
        Ok(JournalPhase::Committed)
    }

    /// Returns only unfinished actions. No automatic retry is executed here.
    /// On a lost ACK the item may still be Staged, so *both* Staged and
    /// Applied require remote readback before any further destructive action.
    pub fn recovery_queue(&self) -> JournalResult<Vec<RecoveryItem>> {
        let mut statement = self.connection.prepare(
            "SELECT operation_id FROM sync_operations WHERE phase < 4
             ORDER BY vault_id, document_id, operation_id"
        )?;
        let ids = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for raw in ids {
            let id = raw?.parse().map_err(|_| JournalError::CorruptedRecord)?;
            let record = read_operation(&self.connection, id)?
                .ok_or(JournalError::CorruptedRecord)?;
            let action = if record.intent.action == JournalAction::StageDownload {
                // Local files may have changed outside Akula's in-process
                // File Gate; no retry or commit based on a prior hash.
                match record.phase {
                    JournalPhase::Pending => RecoveryAction::ResumeStaging,
                    JournalPhase::Staged | JournalPhase::Applied | JournalPhase::Verified =>
                        RecoveryAction::InspectBeforeRetry,
                    JournalPhase::Committed => return Err(JournalError::CorruptedRecord),
                }
            } else {
                match record.phase {
                    JournalPhase::Pending => RecoveryAction::ResumeStaging,
                    JournalPhase::Staged | JournalPhase::Applied =>
                        RecoveryAction::InspectBeforeRetry,
                    JournalPhase::Verified => RecoveryAction::RetryCommit,
                    JournalPhase::Committed => return Err(JournalError::CorruptedRecord),
                }
            };
            result.push(RecoveryItem { record, action });
        }
        Ok(result)
    }
}
