//! Invariants for real Blue W3 v2 verification, sender key, scoped route,
//! recipient W4 signer, and SQLite commit under the SAME revocation fence.
//! Fixture key/route provisioning is intentionally test-only.

use super::*;
use crate::messaging::w4::{verify_receipt, key_ledger::W4TrustLedger};
use akula_blue_w3::{new_recipient, seal_v2, SignedHeader};
use ed25519_dalek::SigningKey;
use ring::signature::Ed25519KeyPair;
use tempfile::TempDir;

const SENDER_EPOCH: u64 = 7;
const RECIPIENT_EPOCH: u64 = 5;
const NOW_MS: u64 = 100;
const EXPIRY_MS: u64 = 9_000;

struct Fixture {
    temp: TempDir,
    inbox: W4Inbox,
    sender: DeviceId,
    recipient: DeviceId,
    sender_signer: SigningKey,
    recipient_signer: Ed25519KeyPair,
    recipient_secret: RecipientSecret,
    recipient_public: akula_blue_w3::RecipientPublic,
}

impl Fixture {
    fn new(allow: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let inbox = W4Inbox::open(&temp.path().join("messages.sqlite")).unwrap();
        let sender = DeviceId::from_uuid(uuid::Uuid::from_bytes([3; 16]));
        let recipient = DeviceId::from_uuid(uuid::Uuid::from_bytes([2; 16]));
        let sender_signer = SigningKey::from_bytes(&[3; 32]);
        let recipient_signer = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let (recipient_secret, recipient_public) = new_recipient();
        let mut this = Self {
            temp,
            inbox,
            sender,
            recipient,
            sender_signer,
            recipient_signer,
            recipient_secret,
            recipient_public,
        };
        let sender_key = this.sender_signer.verifying_key().to_bytes();
        let recipient_key: [u8; 32] = this
            .recipient_signer
            .public_key()
            .as_ref()
            .try_into()
            .unwrap();
        this.seed_key(this.sender, SENDER_EPOCH, sender_key);
        this.seed_key(this.recipient, RECIPIENT_EPOCH, recipient_key);
        if allow {
            assert_eq!(
                { let approved = this.approval(SENDER_EPOCH, 11); this.inbox.allow_route(&approved) }.unwrap(),
                RouteOutcome::Allowed
            );
        }
        this
    }

    /// Direct fixture database write is ONLY possible inside this cfg(test)
    /// child module. Production enrollment uses private ApprovedEnrollment
    /// tokens in W4TrustLedger and never accepts a network-provided signer.
    fn seed_key(&mut self, device: DeviceId, epoch: u64, key: [u8; 32]) {
        self.inbox
            .db
            .execute(
                "INSERT INTO messaging_w4_trusted_devices
                 (device,epoch,ed25519,evidence,revoked)
                 VALUES(?1,?2,?3,?4,0)",
                params![
                    device.as_uuid().as_bytes().as_slice(),
                    epoch as i64,
                    key.as_slice(),
                    [44u8; 32].as_slice(),
                ],
            )
            .unwrap();
    }

    fn approval(&self, sender_epoch: u64, evidence: u8) -> ApprovedInboxRoute {
        ApprovedInboxRoute {
            sender: self.sender,
            recipient: self.recipient,
            sender_epoch,
            evidence_digest: [evidence; 32],
        }
    }

    fn revocation(&self, sender_epoch: u64, evidence: u8) -> ApprovedInboxRevocation {
        ApprovedInboxRevocation {
            sender: self.sender,
            recipient: self.recipient,
            sender_epoch,
            evidence_digest: [evidence; 32],
        }
    }

    fn wire(&self, id: u8, payload: &[u8]) -> Vec<u8> {
        seal_v2(
            &self.sender_signer,
            &self.recipient_public,
            SignedHeader {
                sender_id: self.sender.as_uuid().into_bytes(),
                recipient_id: self.recipient.as_uuid().into_bytes(),
                message_id: [id; 16],
                epoch: SENDER_EPOCH,
                expires_at_ms: EXPIRY_MS,
            },
            payload,
        )
        .unwrap()
    }

    fn accept(&mut self, wire: &[u8]) -> Result<InboundResult, Error> {
        self.inbox.accept_blue_packet(
            wire,
            &self.recipient_secret,
            self.recipient,
            &self.recipient_signer,
            RECIPIENT_EPOCH,
            NOW_MS,
        )
    }
}

#[test]
fn genuine_blue_packet_commits_ciphertext_and_real_recipient_signed_ack() {
    let mut f = Fixture::new(true);
    let wire = f.wire(11, b"Akula authenticated W3");
    let result = f.accept(&wire).unwrap();
    assert_eq!(result.status, super::super::InboundStatus::Stored);
    assert_eq!(f.inbox.row_count().unwrap(), 1);
    let original_packet: Vec<u8> = f
        .inbox
        .db
        .query_row("SELECT ciphertext FROM messaging_w4_inbox", [], |r| r.get(0))
        .unwrap();
    assert_eq!(original_packet, wire);

    let ledger = W4TrustLedger::open(&f.temp.path().join("messages.sqlite")).unwrap();
    let signed = verify_receipt(&result.signed_receipt, &ledger).unwrap();
    assert_eq!(signed.recipient(), f.recipient);
    assert_eq!(signed.message(), [11; 16]);
    assert_eq!(signed.epoch(), RECIPIENT_EPOCH);
    assert_eq!(signed.digest(), *blake3::hash(&wire).as_bytes());

    // Network duplicate after SQLite commit uses the exact signed receipt.
    let duplicate = f.accept(&wire).unwrap();
    assert_eq!(duplicate.status, super::super::InboundStatus::Duplicate);
    assert_eq!(duplicate.signed_receipt, result.signed_receipt);
    assert_eq!(f.inbox.row_count().unwrap(), 1);

    // Durable receipt survives close/reopen, no re-signing on duplicate.
    let mut restarted = W4Inbox::open(&f.temp.path().join("messages.sqlite")).unwrap();
    let again = restarted
        .accept_blue_packet(
            &wire,
            &f.recipient_secret,
            f.recipient,
            &f.recipient_signer,
            RECIPIENT_EPOCH,
            NOW_MS,
        )
        .unwrap();
    assert_eq!(again.signed_receipt, result.signed_receipt);
}

#[test]
fn no_route_no_sender_key_and_wrong_signer_never_yield_ack() {
    let mut f = Fixture::new(false);
    let wire = f.wire(11, b"blocked");
    assert!(matches!(f.accept(&wire), Err(Error::Unauthorized)));
    assert_eq!(f.inbox.row_count().unwrap(), 0);
    assert_eq!(
        { let approved = f.approval(SENDER_EPOCH, 11); f.inbox.allow_route(&approved) }.unwrap(),
        RouteOutcome::Allowed
    );
    f.inbox
        .db
        .execute(
            "DELETE FROM messaging_w4_trusted_devices WHERE device=?1",
            params![f.sender.as_uuid().as_bytes().as_slice()],
        )
        .unwrap();
    assert!(matches!(f.accept(&wire), Err(Error::Unauthorized)));
    f.seed_key(f.sender, SENDER_EPOCH, f.sender_signer.verifying_key().to_bytes());

    let imposter = Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap();
    assert!(matches!(
        f.inbox.accept_blue_packet(
            &wire,
            &f.recipient_secret,
            f.recipient,
            &imposter,
            RECIPIENT_EPOCH,
            NOW_MS
        ),
        Err(Error::Unauthorized)
    ));
    assert_eq!(f.inbox.row_count().unwrap(), 0);
}

#[test]
fn revocation_after_crypto_before_sqlite_commit_is_enforced() {
    let mut f = Fixture::new(true);
    let wire = f.wire(11, b"race");
    let key_view = SqliteSenderSnapshot {
        db: &f.inbox.db,
        observed: Cell::new(None),
    };
    let verified = blue::open_verified(
        &wire,
        OpenContext {
            secret: &f.recipient_secret,
            recipient_id: f.recipient.as_uuid().into_bytes(),
            now_ms: NOW_MS,
            trusted_sender_keys: &key_view,
        },
    )
    .unwrap();
    let witnessed_key = key_view.observed.get().unwrap();
    // A host-authorized revocation that commits in the verification/commit
    // window is visible inside the immediate acceptance transaction.
    assert_eq!(
        { let approved = f.revocation(SENDER_EPOCH, 91); f.inbox.revoke_route(&approved) }.unwrap(),
        RouteOutcome::Revoked
    );
    assert!(matches!(
        f.inbox.commit_blue_verified(
            verified,
            witnessed_key,
            f.recipient,
            &f.recipient_signer,
            RECIPIENT_EPOCH,
            NOW_MS,
        ),
        Err(Error::Unauthorized)
    ));
    assert_eq!(f.inbox.row_count().unwrap(), 0);
    assert!(matches!(
        { let approved = f.approval(SENDER_EPOCH, 99); f.inbox.allow_route(&approved) },
        Err(Error::Unauthorized)
    ));
    assert!(matches!(f.accept(&wire), Err(Error::Unauthorized)));
}

#[test]
fn key_revocation_blocks_even_replay_of_previously_committed_receipt() {
    let mut f = Fixture::new(true);
    let wire = f.wire(11, b"old receipt");
    let result = f.accept(&wire).unwrap();
    assert_eq!(f.inbox.row_count().unwrap(), 1);
    f.inbox
        .db
        .execute(
            "UPDATE messaging_w4_trusted_devices SET revoked=1 WHERE device=?1",
            params![f.sender.as_uuid().as_bytes().as_slice()],
        )
        .unwrap();
    assert!(matches!(f.accept(&wire), Err(Error::Unauthorized)));
    assert_eq!(f.inbox.row_count().unwrap(), 1);
    assert_eq!(result.signed_receipt.len(), super::super::super::RECEIPT_LEN);
}

#[test]
fn invalid_ciphertext_and_expiry_never_store() {
    let mut f = Fixture::new(true);
    let wire = f.wire(11, b"signed");
    let mut tampered = wire.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert!(matches!(
        f.accept(&tampered),
        Err(Error::InvalidSignature)
    ));
    let attacker = SigningKey::from_bytes(&[8; 32]);
    let forged = seal_v2(
        &attacker,
        &f.recipient_public,
        SignedHeader {
            sender_id: f.sender.as_uuid().into_bytes(),
            recipient_id: f.recipient.as_uuid().into_bytes(),
            message_id: [12; 16],
            epoch: SENDER_EPOCH,
            expires_at_ms: EXPIRY_MS,
        },
        b"forged",
    )
    .unwrap();
    assert!(matches!(
        f.accept(&forged),
        Err(Error::InvalidSignature)
    ));
    assert!(matches!(
        f.inbox.accept_blue_packet(
            &wire,
            &f.recipient_secret,
            f.recipient,
            &f.recipient_signer,
            RECIPIENT_EPOCH,
            EXPIRY_MS,
        ),
        Err(Error::Expired)
    ));
    assert_eq!(f.inbox.row_count().unwrap(), 0);
}

#[test]
fn failed_disk_insert_cannot_emit_w4_recipient_receipt() {
    let mut f = Fixture::new(true);
    let wire = f.wire(11, b"fail closed");
    f.inbox
        .db
        .execute_batch(
            "CREATE TRIGGER deny_all_inserts BEFORE INSERT ON messaging_w4_inbox
             BEGIN SELECT RAISE(ABORT,'test disk failure'); END;",
        )
        .unwrap();
    assert!(matches!(f.accept(&wire), Err(Error::Database(_))));
    assert_eq!(f.inbox.row_count().unwrap(), 0);
    f.inbox
        .db
        .execute_batch("DROP TRIGGER deny_all_inserts")
        .unwrap();
    assert_eq!(f.accept(&wire).unwrap().status, super::super::InboundStatus::Stored);
}

#[test]
fn host_route_epoch_is_monotonic_and_approval_evidence_is_not_replayable() {
    let mut f = Fixture::new(true);
    assert_eq!(
        { let approved = f.approval(SENDER_EPOCH, 11); f.inbox.allow_route(&approved) }.unwrap(),
        RouteOutcome::Duplicate
    );
    assert!(matches!(
        { let approved = f.approval(SENDER_EPOCH, 12); f.inbox.allow_route(&approved) },
        Err(Error::Conflict)
    ));
    assert_eq!(
        { let approved = f.revocation(SENDER_EPOCH, 99); f.inbox.revoke_route(&approved) }.unwrap(),
        RouteOutcome::Revoked
    );
    assert!(matches!(
        { let approved = f.approval(SENDER_EPOCH, 11); f.inbox.allow_route(&approved) },
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        { let approved = f.approval(SENDER_EPOCH + 1, 12); f.inbox.allow_route(&approved) },
        Err(Error::Unauthorized)
    ));
    assert_eq!(f.inbox.row_count().unwrap(), 0);
}
