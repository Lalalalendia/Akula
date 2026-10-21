//! E3e: receiver-side one-shot challenge for an explicitly approved Vault join.
//!
//! This deliberately adds NO new database, persisted nonce, S3 writer or
//! approval UI. A pending session lives only in the receiver's process.
//! The owner key set and genesis pins MUST still be verified by the human
//! over an authenticated channel outside untrusted S3. The older E3c method
//! remains an isolated laboratory and is NOT a production enrollment API.
//!
//! The one-shot state is spent BEFORE verifying the supplied invitation.
//! A failed attempt requires a new challenge. A restart discards the session,
//! so an old signed offer is never reusable through this API after restart.

use super::*;
use super::super::super::{IndependentlyApprovedJoin, PublisherPolicyV1};
#[cfg(feature = "signed-vault-join-successor-integration-lab")]
use super::super::super::{AwsS3ProbeClient, PinnedJoinError};
use ring::rand::{SecureRandom, SystemRandom};
use std::time::{Duration, Instant};

const CHALLENGE_BYTES: usize = 32;
const RECEIVER_SESSION_LIFETIME: Duration = Duration::from_secs(300);
const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Debug)]
pub enum ReceiverJoinSessionError {
    EntropyUnavailable,
    SessionClockUnavailable,
    AlreadyConsumed,
    Expired,
    WrongLocalScope,
    WrongChallenge,
    Join(AtomicJoinError),
    #[cfg(feature = "signed-vault-join-successor-integration-lab")]
    LiveYandexRead(PinnedJoinError),
    #[cfg(feature = "signed-vault-manual-owner-review-lab")]
    ManualOwnerReview(super::super::super::IndependentOwnerFingerprintError),
}

#[derive(Debug)]
pub struct ReceiverJoinSession {
    // A public challenge, not an authorization secret. Never persisted.
    challenge_hex: String,
    profile_id: String,
    binding_fingerprint: String,
    head_key: String,
    expires_at: Instant,
    consumed: bool,
}

impl ReceiverJoinSession {
    /// Start a single interactive join attempt on the RECEIVING device.
    /// The host must show the challenge to the legitimate owner using a
    /// separately authenticated channel. S3 must never provide the trusted
    /// owner keys or the independent root+policy pins.
    pub fn issue(scope: &S3RemoteAnchorScope) -> Result<Self, ReceiverJoinSessionError> {
        let mut challenge = [0u8; CHALLENGE_BYTES];
        SystemRandom::new().fill(&mut challenge)
            .map_err(|_| ReceiverJoinSessionError::EntropyUnavailable)?;
        // Reject a pathological all-zero output rather than mint an E3a
        // challenge which the verifier is required to reject.
        if challenge.iter().all(|b| *b == 0) {
            return Err(ReceiverJoinSessionError::EntropyUnavailable);
        }
        let mut challenge_hex = String::with_capacity(CHALLENGE_BYTES * 2);
        for byte in challenge {
            challenge_hex.push(HEX[(byte >> 4) as usize] as char);
            challenge_hex.push(HEX[(byte & 0x0f) as usize] as char);
        }
        let expires_at = Instant::now()
            .checked_add(RECEIVER_SESSION_LIFETIME)
            .ok_or(ReceiverJoinSessionError::SessionClockUnavailable)?;
        Ok(Self {
            challenge_hex,
            profile_id: scope.profile_id().to_owned(),
            binding_fingerprint: scope.binding_fingerprint().to_owned(),
            head_key: scope.keyspace().current_head_key(),
            expires_at,
            consumed: false,
        })
    }

    /// Owned copy lets a caller construct an IndependentlyApprovedJoin
    /// without borrowing the session while enroll_once mutably consumes it.
    /// Revealing the challenge does not reveal or authorize a signing key.
    pub fn challenge_hex(&self) -> String {
        self.challenge_hex.clone()
    }

    /// Consume this local challenge exactly once, including on any rejected
    /// signature, wrong scope, expired session, or SQLite failure.
    ///
    /// The inner E3c journal still enforces independent root+policy pins,
    /// E3a signature/threshold, complete pinned V2 read, and atomic SQLite
    /// root+policy admission. This wrapper does NOT independently attest the
    /// human provenance of approved.owner_keys/pins, and is not a wire codec.
    pub fn enroll_once(
        &mut self,
        journal: &mut S3V2TrustedAnchorJournal,
        scope: &S3RemoteAnchorScope,
        root: &S3PackedHeadV2,
        root_etag: &str,
        completed_read: &S3PinnedV2ReadPreview,
        policy: &PublisherPolicyV1,
        signed_offer: &DecodedDsseEnvelope<'_>,
        approved: &IndependentlyApprovedJoin<'_>,
    ) -> Result<AtomicJoinOutcome, ReceiverJoinSessionError> {
        self.consume_for_approved_scope(scope, approved)?;
        journal.enroll_signed_pinned_genesis_atomically(
            scope, root, root_etag, completed_read, policy, signed_offer,
            approved,
        ).map_err(ReceiverJoinSessionError::Join)
    }


    // Shared gate for both synthetic laboratory admission and the network
    // entrypoint. Never let the remote call happen before this spends the
    // challenge and checks the receiver's independent local scope.
    fn consume_for_approved_scope(
        &mut self,
        scope: &S3RemoteAnchorScope,
        approved: &IndependentlyApprovedJoin<'_>,
    ) -> Result<(), ReceiverJoinSessionError> {
        if std::mem::replace(&mut self.consumed, true) {
            return Err(ReceiverJoinSessionError::AlreadyConsumed);
        }
        if Instant::now() >= self.expires_at {
            return Err(ReceiverJoinSessionError::Expired);
        }
        if scope.profile_id() != self.profile_id
            || scope.binding_fingerprint() != self.binding_fingerprint
            || scope.keyspace().current_head_key() != self.head_key
        {
            return Err(ReceiverJoinSessionError::WrongLocalScope);
        }
        if approved.recipient_challenge_hex != self.challenge_hex {
            return Err(ReceiverJoinSessionError::WrongChallenge);
        }
        Ok(())
    }

    /// E3g: perform the *actual* bounded Yandex V2 GET chain after a fresh
    /// receiver challenge and independent owner/root/policy approval.
    /// The private pinned receipt comes only from the inspected transport,
    /// then E3c atomically records the exact root and publisher policy.
    ///
    /// This method does not infer account identity from an S3 response:
    /// the caller must independently authenticate the account binding and
    /// owner's key set. No File Gate, user note write, S3 writer or Sync.
    #[cfg(feature = "signed-vault-join-successor-integration-lab")]
    pub async fn enroll_from_yandex_read_once(
        &mut self,
        client: &AwsS3ProbeClient,
        journal: &mut S3V2TrustedAnchorJournal,
        scope: &S3RemoteAnchorScope,
        root: &S3PackedHeadV2,
        policy: &PublisherPolicyV1,
        signed_offer: &DecodedDsseEnvelope<'_>,
        approved: &IndependentlyApprovedJoin<'_>,
    ) -> Result<AtomicJoinOutcome, ReceiverJoinSessionError> {
        self.consume_for_approved_scope(scope, approved)?;
        Self::finish_verified_yandex_join(
            client, journal, scope, root, policy, signed_offer, approved,
        ).await
    }

    /// E3h: the host MUST obtain `entered_out_of_band_hex` from a separate,
    /// authenticated owner-controlled channel. It must NEVER copy or prefill
    /// the expected value from candidate S3 objects or invitation payload.
    /// A wrong or missing 256-bit fingerprint burns the session and cannot
    /// perform any S3 request or SQLite trust registration.
    #[cfg(feature = "signed-vault-manual-owner-review-lab")]
    pub async fn enroll_after_independent_owner_review_once(
        &mut self,
        client: &AwsS3ProbeClient,
        journal: &mut S3V2TrustedAnchorJournal,
        scope: &S3RemoteAnchorScope,
        root: &S3PackedHeadV2,
        policy: &PublisherPolicyV1,
        signed_offer: &DecodedDsseEnvelope<'_>,
        approved: &IndependentlyApprovedJoin<'_>,
        entered_out_of_band_hex: &str,
    ) -> Result<AtomicJoinOutcome, ReceiverJoinSessionError> {
        self.consume_for_approved_scope(scope, approved)?;
        super::super::super::s3_owner_manual_fingerprint_lab::
            verify_independently_entered_owner_fingerprint(
                approved, entered_out_of_band_hex,
            ).map_err(ReceiverJoinSessionError::ManualOwnerReview)?;
        Self::finish_verified_yandex_join(
            client, journal, scope, root, policy, signed_offer, approved,
        ).await
    }

    // Exactly ONE internal S3-to-SQLite path under both session entrypoints.
    // Caller-owned ETag/pinned receipts are never substituted for the SDK's
    // actual bounded read; E3c remains the authority for atomic enrollment.
    #[cfg(feature = "signed-vault-join-successor-integration-lab")]
    async fn finish_verified_yandex_join(
        client: &AwsS3ProbeClient,
        journal: &mut S3V2TrustedAnchorJournal,
        scope: &S3RemoteAnchorScope,
        root: &S3PackedHeadV2,
        policy: &PublisherPolicyV1,
        signed_offer: &DecodedDsseEnvelope<'_>,
        approved: &IndependentlyApprovedJoin<'_>,
    ) -> Result<AtomicJoinOutcome, ReceiverJoinSessionError> {
        let (etag, pinned_read) =
            super::super::super::s3_join_pinned_lab::verify_yandex_join_for_atomic_session(
                client, scope, root, policy, approved, signed_offer,
            ).await.map_err(ReceiverJoinSessionError::LiveYandexRead)?;
        journal.enroll_signed_pinned_genesis_atomically(
            scope, root, &etag, &pinned_read, policy, signed_offer, approved,
        ).map_err(ReceiverJoinSessionError::Join)
    }

    #[cfg(test)]
    pub(crate) fn expire_for_test(&mut self) {
        self.expires_at = Instant::now().checked_sub(Duration::from_secs(1))
            .expect("monotonic clock supports one-second subtraction");
    }
}
