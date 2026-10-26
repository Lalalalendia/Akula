//! Fail-closed, pre-publication object receipts for the isolated Dropbox lab.
//!
//! This module has no remote writer, file-system writer, SQLite baseline,
//! manifest verifier, File Gate authority or access to native documents.
//! In particular, an object being verified NEVER means its vault head was
//! published. A future coordinator must provide separate durable head proof.

use crate::{DropboxRevision, ObjectKey, Scope};
use serde::{Deserialize, Serialize};

/// Bounded, in-memory inspection only. Large objects require a later
/// independently tested streaming verifier.
pub const MAX_READBACK_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Prepared,
    AwaitingReadback,
    VerifiedUnpublished,
    Held,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiptError {
    InvalidIntent,
    InvalidPhase,
    InvalidRecord,
    ScopeChanged,
    ObjectChanged,
    InvalidProviderIdentity,
    OversizedReadback,
    IntegrityMismatch,
}

/// A read-only candidate, not permission to publish the vault head or
/// advance a confirmed document baseline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicationCandidate {
    pub operation_id: String,
    pub scope: Scope,
    pub object_key: ObjectKey,
    pub expected_blake3: String,
    pub provider_file_id: String,
    pub provider_revision: DropboxRevision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnpublishedObjectReceipt {
    operation_id: String,
    scope: Scope,
    object_key: ObjectKey,
    expected_blake3: String,
    phase: Phase,
    provider_file_id: Option<String>,
    provider_revision: Option<DropboxRevision>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireReceipt {
    version: u8,
    operation_id: String,
    account_id: String,
    root_namespace_id: String,
    vault_id: String,
    profile_id: String,
    object_key: String,
    expected_blake3: String,
    phase: Phase,
    provider_file_id: Option<String>,
    provider_revision: Option<String>,
}

fn is_lower_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn is_lower_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| {
        if [8, 13, 18, 23].contains(&i) {
            *c == b'-'
        } else {
            c.is_ascii_digit() || (b'a'..=b'f').contains(c)
        }
    })
}

fn valid_native_file_id(id: &str) -> bool {
    id.strip_prefix("id:").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.len() <= 256
            && suffix.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    })
}

impl UnpublishedObjectReceipt {
    pub fn new(
        scope: Scope,
        operation_id: &str,
        object_key: ObjectKey,
        expected_blake3: &str,
    ) -> Result<Self, ReceiptError> {
        if !is_lower_uuid(operation_id)
            || !is_lower_hex64(expected_blake3)
            || object_key.is_head()
            || object_key.as_str().strip_prefix("blobs/")
                .is_some_and(|digest| digest != expected_blake3)
        {
            return Err(ReceiptError::InvalidIntent);
        }
        Ok(Self {
            operation_id: operation_id.to_owned(),
            scope,
            object_key,
            expected_blake3: expected_blake3.to_owned(),
            phase: Phase::Prepared,
            provider_file_id: None,
            provider_revision: None,
        })
    }

    pub fn phase(&self) -> Phase { self.phase }

    /// MUST be made durable in the future real journal BEFORE the network
    /// request starts. Only the very first intent is eligible for a send.
    pub fn mark_attempt_started(&mut self) -> Result<(), ReceiptError> {
        if self.phase != Phase::Prepared {
            return Err(ReceiptError::InvalidPhase);
        }
        self.phase = Phase::AwaitingReadback;
        Ok(())
    }

    /// A lost response, 5xx or timeout can hide a successful upload.
    /// This intentionally does not re-arm a second attempt.
    pub fn mark_response_uncertain(&mut self) -> Result<(), ReceiptError> {
        if self.phase != Phase::AwaitingReadback {
            return Err(ReceiptError::InvalidPhase);
        }
        Ok(())
    }

    /// Provider-reported conflict means a different operation may own the
    /// object. An explicit new plan is required, never blind overwrite.
    pub fn hold_after_conflict(&mut self) {
        self.phase = Phase::Held;
        self.provider_file_id = None;
        self.provider_revision = None;
    }

    /// Inspect the exact requested logical object after the write.
    /// The digest comes from actual bytes (BLAKE3), NOT Dropbox content_hash.
    /// Even success grants only VerifiedUnpublished, never a document baseline.
    pub fn inspect_bounded_readback(
        &mut self,
        observed_scope: &Scope,
        observed_object: &ObjectKey,
        native_file_id: &str,
        provider_revision: DropboxRevision,
        bytes: &[u8],
    ) -> Result<(), ReceiptError> {
        if self.phase != Phase::AwaitingReadback {
            return Err(ReceiptError::InvalidPhase);
        }
        let failure = if &self.scope != observed_scope {
            Some(ReceiptError::ScopeChanged)
        } else if &self.object_key != observed_object {
            Some(ReceiptError::ObjectChanged)
        } else if !valid_native_file_id(native_file_id) {
            Some(ReceiptError::InvalidProviderIdentity)
        } else if bytes.len() > MAX_READBACK_BYTES {
            Some(ReceiptError::OversizedReadback)
        } else if blake3::hash(bytes).to_hex().as_str() != self.expected_blake3.as_str() {
            Some(ReceiptError::IntegrityMismatch)
        } else {
            None
        };
        if let Some(reason) = failure {
            self.hold_after_conflict();
            return Err(reason);
        }
        self.phase = Phase::VerifiedUnpublished;
        self.provider_file_id = Some(native_file_id.to_owned());
        self.provider_revision = Some(provider_revision);
        Ok(())
    }

    /// Ready for separate archive-chain inspection, NOT proof of publication.
    pub fn candidate(&self) -> Option<PublicationCandidate> {
        if self.phase != Phase::VerifiedUnpublished {
            return None;
        }
        Some(PublicationCandidate {
            operation_id: self.operation_id.clone(),
            scope: self.scope.clone(),
            object_key: self.object_key.clone(),
            expected_blake3: self.expected_blake3.clone(),
            provider_file_id: self.provider_file_id.clone()?,
            provider_revision: self.provider_revision.clone()?,
        })
    }

    /// Portable journal record only. Callers are responsible for durable
    /// storage. This function deliberately performs no filesystem operations.
    pub fn encode(&self) -> Vec<u8> {
        let record = WireReceipt {
            version: 1,
            operation_id: self.operation_id.clone(),
            account_id: self.scope.account_id().to_owned(),
            root_namespace_id: self.scope.root_namespace_id().to_owned(),
            vault_id: self.scope.vault_id().to_owned(),
            profile_id: self.scope.profile_id().to_owned(),
            object_key: self.object_key.as_str().to_owned(),
            expected_blake3: self.expected_blake3.clone(),
            phase: self.phase,
            provider_file_id: self.provider_file_id.clone(),
            provider_revision: self.provider_revision.as_ref().map(|r| r.as_str().to_owned()),
        };
        serde_json::to_vec(&record).expect("typed receipt can be serialized")
    }

    /// A previously stored record is never trusted as a live readback proof.
    /// After restart, even a previously verified object needs a NEW readback.
    /// No persisted state can re-enable a blind network write.
    pub fn recover(bytes: &[u8], trusted_intent: &Self) -> Result<Self, ReceiptError> {
        let wire: WireReceipt =
            serde_json::from_slice(bytes).map_err(|_| ReceiptError::InvalidRecord)?;
        if wire.version != 1 {
            return Err(ReceiptError::InvalidRecord);
        }
        let scope = Scope::new(
            &wire.account_id,
            &wire.root_namespace_id,
            &wire.vault_id,
            &wire.profile_id,
        ).map_err(|_| ReceiptError::InvalidRecord)?;
        if &scope != &trusted_intent.scope {
            return Err(ReceiptError::ScopeChanged);
        }
        let key = ObjectKey::parse(&wire.object_key)
            .map_err(|_| ReceiptError::InvalidRecord)?;
        let mut receipt = Self::new(scope, &wire.operation_id, key, &wire.expected_blake3)
            .map_err(|_| ReceiptError::InvalidRecord)?;
        if receipt.operation_id != trusted_intent.operation_id
            || receipt.object_key != trusted_intent.object_key
            || receipt.expected_blake3 != trusted_intent.expected_blake3
        {
            return Err(ReceiptError::InvalidRecord);
        }
        let has_both = wire.provider_file_id.is_some() && wire.provider_revision.is_some();
        let has_neither = wire.provider_file_id.is_none() && wire.provider_revision.is_none();
        if wire.phase == Phase::VerifiedUnpublished {
            if !has_both || !valid_native_file_id(wire.provider_file_id.as_deref().unwrap()) {
                return Err(ReceiptError::InvalidRecord);
            }
            DropboxRevision::parse(wire.provider_revision.as_deref().unwrap())
                .map_err(|_| ReceiptError::InvalidRecord)?;
        } else if !has_neither {
            return Err(ReceiptError::InvalidRecord);
        }
        receipt.phase = if wire.phase == Phase::Held {
            Phase::Held
        } else {
            Phase::AwaitingReadback
        };
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> Scope {
        Scope::new(
            "dbid:alice", "12345",
            "12345678-1234-1234-1234-123456789abc", "home",
        ).unwrap()
    }
    fn key() -> ObjectKey {
        ObjectKey::parse(&format!("blobs/{}", blake3::hash(b"example Markdown").to_hex())).unwrap()
    }
    fn new_receipt() -> UnpublishedObjectReceipt {
        let bytes = b"example Markdown";
        UnpublishedObjectReceipt::new(
            scope(), "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            key(), &blake3::hash(bytes).to_hex().to_string(),
        ).unwrap()
    }

    #[test]
    fn uploaded_bytes_are_only_verified_unpublished() {
        let mut r = new_receipt();
        r.mark_attempt_started().unwrap();
        r.inspect_bounded_readback(&scope(), &key(), "id:abc",
            DropboxRevision::parse("R1").unwrap(), b"example Markdown").unwrap();
        assert_eq!(r.phase(), Phase::VerifiedUnpublished);
        assert_eq!(r.candidate().unwrap().expected_blake3,
            blake3::hash(b"example Markdown").to_hex().to_string());
        assert!(r.mark_attempt_started().is_err());
    }

    #[test]
    fn content_mismatch_fails_closed() {
        let mut r = new_receipt();
        r.mark_attempt_started().unwrap();
        assert_eq!(r.inspect_bounded_readback(&scope(), &key(), "id:abc",
            DropboxRevision::parse("R1").unwrap(), b"something entirely"),
            Err(ReceiptError::IntegrityMismatch));
        assert_eq!(r.phase(), Phase::Held);
        assert!(r.candidate().is_none());
    }

    #[test]
    fn restart_requires_readback_and_never_rearms_upload() {
        let mut r = new_receipt();
        r.mark_attempt_started().unwrap();
        r.inspect_bounded_readback(&scope(), &key(), "id:abc",
            DropboxRevision::parse("R1").unwrap(), b"example Markdown").unwrap();
        let recovered = UnpublishedObjectReceipt::recover(&r.encode(), &new_receipt()).unwrap();
        assert_eq!(recovered.phase(), Phase::AwaitingReadback);
        assert!(recovered.candidate().is_none());
        let mut recovered = recovered;
        assert_eq!(recovered.mark_attempt_started(), Err(ReceiptError::InvalidPhase));
    }
}
