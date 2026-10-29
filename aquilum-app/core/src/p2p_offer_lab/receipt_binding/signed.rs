//! AK-P2P-B3 signed receipt provenance, offline verifier only.
//!
//! Not a sender path, UI/open/delete permission, transport grant or durable
//! anti-replay journal. The caller must independently pin both endpoint keys,
//! rehash the actual opened Inbox file and retain a protected one-use journal.
//! This pure module cannot establish physical user consent or OS file custody.
use super::ObservedLocalReceipt;
use crate::p2p_offer_lab::{OfferError, OfferV1, PeerBinding};
use crate::sync::DeviceId;
use ring::signature::{UnparsedPublicKey, ED25519};
use uuid::Uuid;

pub mod journal;

// Non-shipping issuance is compiled only with the separately opted-in OS key.
#[cfg(feature = "p2p-device-identity")]
pub mod issuance;

const MAGIC: &[u8; 8] = b"AKP2RCP2";
const VERSION: u16 = 2;
const BODY_BYTES: usize = 235;
const SIGNATURE_BYTES: usize = 64;
pub const RECORD_BYTES: usize = BODY_BYTES + 2 * SIGNATURE_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProofError {
    BadLength,
    WrongVersion,
    WrongContext,
    WrongSelection,
    WrongReceipt,
    InvalidTime,
    InvalidOffer(OfferError),
    BadSignature,
}

/// A verified, non-authorizing result. Deliberately contains no filename,
/// MIME type, Vault destination or permission to consume the signed proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckedColdReceipt {
    offer_id: Uuid,
    receipt_id: Uuid,
    selected_index: u8,
    manifest_commitment: [u8; 32],
    proof_digest: [u8; 32],
}

/// A V2 receipt is offer-scoped and cannot reuse the legacy content-only ID.
/// Determinism does not constitute a durable consumed/replay ledger.
pub fn receipt_id_v2(offer: &OfferV1, selected_index: u8) -> Result<Uuid, ProofError> {
    let file = offer
        .files
        .get(usize::from(selected_index))
        .ok_or(ProofError::WrongSelection)?;
    let mut hash = blake3::Hasher::new_derive_key("Akula signed P2P Offer receipt ID v2");
    hash.update(offer.offer_id.as_bytes());
    hash.update(&offer.nonce);
    // Bind the full Offer, including its display-only label and enrollment IDs.
    hash.update(&offer.commitment());
    hash.update(&[selected_index]);
    hash.update(&offer.sender.endpoint_id);
    hash.update(&offer.receiver.endpoint_id);
    hash.update(&file.bytes.to_be_bytes());
    hash.update(&file.blake3_root);
    let digest = hash.finalize();
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(&digest.as_bytes()[..16]);
    uuid[6] = (uuid[6] & 0x0f) | 0x80;
    uuid[8] = (uuid[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(uuid))
}

fn take<const N: usize>(record: &[u8], cursor: &mut usize) -> [u8; N] {
    let mut value = [0u8; N];
    value.copy_from_slice(&record[*cursor..*cursor + N]);
    *cursor += N;
    value
}

/// Verify a versioned, dual-signed byte record after a cold process restart.
/// The receiver's Ed25519 signature covers the sender's signature and the
/// identical fixed-length transcript. Both keys MUST come from independent
/// enrollment, not from record bytes. The accepted time is a signed claim;
/// creation under a valid live consent and durable one-use state still require
/// separate OS-pinned runtime integration.
///
/// The supplied observed facts MUST come from a freshly rehashed local handle.
/// Neither raw SQLite, network metadata nor sender-provided labels qualify.
pub fn verify_cold_receipt_v2(
    record: &[u8],
    stored_offer: &OfferV1,
    pinned_sender: PeerBinding,
    pinned_receiver: PeerBinding,
    observed: ObservedLocalReceipt,
    now: u64,
) -> Result<CheckedColdReceipt, ProofError> {
    if record.len() != RECORD_BYTES {
        return Err(ProofError::BadLength);
    }
    if &record[..MAGIC.len()] != MAGIC {
        return Err(ProofError::WrongVersion);
    }
    let mut at = MAGIC.len();
    if u16::from_be_bytes(take::<2>(record, &mut at)) != VERSION {
        return Err(ProofError::WrongVersion);
    }
    let offer_id = Uuid::from_bytes(take::<16>(record, &mut at));
    let nonce = take::<16>(record, &mut at);
    let commitment = take::<32>(record, &mut at);
    let index = take::<1>(record, &mut at)[0];
    let receipt_id = Uuid::from_bytes(take::<16>(record, &mut at));
    let sender = PeerBinding {
        device_id: DeviceId::from_uuid(Uuid::from_bytes(take::<16>(record, &mut at))),
        endpoint_id: take::<32>(record, &mut at),
    };
    let receiver = PeerBinding {
        device_id: DeviceId::from_uuid(Uuid::from_bytes(take::<16>(record, &mut at))),
        endpoint_id: take::<32>(record, &mut at),
    };
    let bytes = u64::from_be_bytes(take::<8>(record, &mut at));
    let digest = take::<32>(record, &mut at);
    let accepted_at = u64::from_be_bytes(take::<8>(record, &mut at));
    debug_assert_eq!(at, BODY_BYTES);

    if sender != pinned_sender
        || receiver != pinned_receiver
        || stored_offer.sender != pinned_sender
        || stored_offer.receiver != pinned_receiver
        || stored_offer.offer_id != offer_id
        || stored_offer.nonce != nonce
        || stored_offer.commitment() != commitment
    {
        return Err(ProofError::WrongContext);
    }
    if now < accepted_at
        || accepted_at < stored_offer.created_unix
        || accepted_at >= stored_offer.expires_unix
    {
        return Err(ProofError::InvalidTime);
    }
    stored_offer
        .validate(accepted_at)
        .map_err(ProofError::InvalidOffer)?;
    let selected = stored_offer
        .files
        .get(usize::from(index))
        .ok_or(ProofError::WrongSelection)?;
    if receipt_id.is_nil()
        || receipt_id_v2(stored_offer, index)? != receipt_id
        || observed.receipt_id != receipt_id
        || observed.sender_endpoint != pinned_sender.endpoint_id
        || observed.receiver_endpoint != pinned_receiver.endpoint_id
        || observed.bytes != bytes
        || observed.blake3_root != digest
        || selected.bytes != bytes
        || selected.blake3_root != digest
    {
        return Err(ProofError::WrongReceipt);
    }
    let sender_signature = &record[BODY_BYTES..BODY_BYTES + SIGNATURE_BYTES];
    let receiver_signature = &record[BODY_BYTES + SIGNATURE_BYTES..];
    UnparsedPublicKey::new(&ED25519, &pinned_sender.endpoint_id)
        .verify(&record[..BODY_BYTES], sender_signature)
        .map_err(|_| ProofError::BadSignature)?;
    UnparsedPublicKey::new(&ED25519, &pinned_receiver.endpoint_id)
        .verify(&record[..BODY_BYTES + SIGNATURE_BYTES], receiver_signature)
        .map_err(|_| ProofError::BadSignature)?;
    Ok(CheckedColdReceipt {
        offer_id,
        receipt_id,
        selected_index: index,
        manifest_commitment: commitment,
        proof_digest: *blake3::hash(record).as_bytes(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p2p_offer_lab::{OfferedFile, SCHEMA};
    use ring::signature::{Ed25519KeyPair, KeyPair};

    const NOW: u64 = 1_800_000_000;

    fn append<const N: usize>(
        buffer: &mut [u8; RECORD_BYTES],
        cursor: &mut usize,
        value: &[u8; N],
    ) {
        buffer[*cursor..*cursor + N].copy_from_slice(value);
        *cursor += N;
    }

    fn fixture() -> (OfferV1, [u8; RECORD_BYTES], ObservedLocalReceipt) {
        let sender_key = Ed25519KeyPair::from_seed_unchecked(&[17; 32]).unwrap();
        let receiver_key = Ed25519KeyPair::from_seed_unchecked(&[29; 32]).unwrap();
        let mut sender_pub = [0u8; 32];
        let mut receiver_pub = [0u8; 32];
        sender_pub.copy_from_slice(sender_key.public_key().as_ref());
        receiver_pub.copy_from_slice(receiver_key.public_key().as_ref());
        let sender = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(1)),
            endpoint_id: sender_pub,
        };
        let receiver = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(2)),
            endpoint_id: receiver_pub,
        };
        let file = OfferedFile {
            label: "signed-report.txt".into(),
            bytes: 5,
            blake3_root: *blake3::hash(b"alpha").as_bytes(),
        };
        let offer = OfferV1 {
            schema: SCHEMA,
            offer_id: Uuid::from_u128(103),
            nonce: [9; 16],
            sender,
            receiver,
            created_unix: NOW,
            expires_unix: NOW + 120,
            files: vec![file.clone()],
        };
        offer.validate(NOW).unwrap();
        let receipt_id = receipt_id_v2(&offer, 0).unwrap();
        let mut record = [0u8; RECORD_BYTES];
        let mut at = 0;
        append(&mut record, &mut at, MAGIC);
        append(&mut record, &mut at, &VERSION.to_be_bytes());
        append(&mut record, &mut at, offer.offer_id.as_bytes());
        append(&mut record, &mut at, &offer.nonce);
        append(&mut record, &mut at, &offer.commitment());
        append(&mut record, &mut at, &[0]);
        append(&mut record, &mut at, receipt_id.as_bytes());
        append(
            &mut record,
            &mut at,
            offer.sender.device_id.as_uuid().as_bytes(),
        );
        append(&mut record, &mut at, &offer.sender.endpoint_id);
        append(
            &mut record,
            &mut at,
            offer.receiver.device_id.as_uuid().as_bytes(),
        );
        append(&mut record, &mut at, &offer.receiver.endpoint_id);
        append(&mut record, &mut at, &file.bytes.to_be_bytes());
        append(&mut record, &mut at, &file.blake3_root);
        append(&mut record, &mut at, &(NOW + 5).to_be_bytes());
        assert_eq!(at, BODY_BYTES);
        let sender_signature = sender_key.sign(&record[..BODY_BYTES]);
        record[at..at + SIGNATURE_BYTES].copy_from_slice(sender_signature.as_ref());
        at += SIGNATURE_BYTES;
        let receiver_signature = receiver_key.sign(&record[..at]);
        record[at..].copy_from_slice(receiver_signature.as_ref());
        let observed = ObservedLocalReceipt {
            receipt_id,
            sender_endpoint: sender_pub,
            receiver_endpoint: receiver_pub,
            bytes: file.bytes,
            blake3_root: file.blake3_root,
        };
        (offer, record, observed)
    }

    fn check(
        offer: &OfferV1,
        record: &[u8],
        observed: ObservedLocalReceipt,
        now: u64,
    ) -> Result<CheckedColdReceipt, ProofError> {
        verify_cold_receipt_v2(record, offer, offer.sender, offer.receiver, observed, now)
    }

    #[test]
    fn signed_two_endpoint_proof_survives_cold_read_after_offer_expiry() {
        let (offer, record, observed) = fixture();
        let archived = record.to_vec();
        let checked = check(&offer, &archived, observed, NOW + 200).unwrap();
        assert_eq!(checked.offer_id, offer.offer_id);
        assert_eq!(checked.receipt_id, observed.receipt_id);
        assert_eq!(checked.selected_index, 0);
        assert_eq!(checked.manifest_commitment, offer.commitment());
    }

    #[test]
    fn unsigned_legacy_receipt_and_malformed_frames_fail_closed() {
        let (offer, record, observed) = fixture();
        assert_eq!(
            check(&offer, &[], observed, NOW + 6),
            Err(ProofError::BadLength)
        );
        assert_eq!(
            check(&offer, &record[..RECORD_BYTES - 1], observed, NOW + 6),
            Err(ProofError::BadLength)
        );
        let mut old = record;
        old[..8].copy_from_slice(b"AKP2ONE1");
        assert_eq!(
            check(&offer, &old, observed, NOW + 6),
            Err(ProofError::WrongVersion)
        );
        let mut unbound = observed;
        unbound.receipt_id = Uuid::from_u128(987);
        assert_eq!(
            check(&offer, &record, unbound, NOW + 6),
            Err(ProofError::WrongReceipt)
        );
    }

    #[test]
    fn tampered_signatures_and_signed_contents_are_refused() {
        let (offer, record, observed) = fixture();
        let mut changed = record;
        changed[BODY_BYTES] ^= 1;
        assert_eq!(
            check(&offer, &changed, observed, NOW + 6),
            Err(ProofError::BadSignature)
        );
        changed = record;
        changed[RECORD_BYTES - 1] ^= 1;
        assert_eq!(
            check(&offer, &changed, observed, NOW + 6),
            Err(ProofError::BadSignature)
        );
        changed = record;
        // Digest still matches the stored Offer, but the signature does not.
        changed[BODY_BYTES - 1] ^= 1;
        assert!(check(&offer, &changed, observed, NOW + 6).is_err());
    }

    #[test]
    fn different_offer_and_receiver_identity_cannot_reuse_proof() {
        let (offer, record, observed) = fixture();
        let mut forged_offer = offer.clone();
        forged_offer.offer_id = Uuid::from_u128(104);
        assert_eq!(
            check(&forged_offer, &record, observed, NOW + 6),
            Err(ProofError::WrongContext)
        );
        forged_offer = offer.clone();
        forged_offer.files[0].label = "different-name.txt".into();
        assert_eq!(
            check(&forged_offer, &record, observed, NOW + 6),
            Err(ProofError::WrongContext)
        );
        let mut wrong = observed;
        wrong.receiver_endpoint = [99; 32];
        assert_eq!(
            check(&offer, &record, wrong, NOW + 6),
            Err(ProofError::WrongReceipt)
        );
        assert_eq!(
            verify_cold_receipt_v2(
                &record,
                &offer,
                offer.sender,
                PeerBinding {
                    device_id: offer.receiver.device_id,
                    endpoint_id: [2; 32],
                },
                observed,
                NOW + 6
            ),
            Err(ProofError::WrongContext)
        );
    }

    #[test]
    fn changed_bytes_hash_receipt_and_clock_fail_closed() {
        let (offer, record, observed) = fixture();
        let mut wrong = observed;
        wrong.bytes += 1;
        assert_eq!(
            check(&offer, &record, wrong, NOW + 6),
            Err(ProofError::WrongReceipt)
        );
        wrong = observed;
        wrong.blake3_root = [0; 32];
        assert_eq!(
            check(&offer, &record, wrong, NOW + 6),
            Err(ProofError::WrongReceipt)
        );
        wrong = observed;
        wrong.receipt_id = Uuid::new_v4();
        assert_eq!(
            check(&offer, &record, wrong, NOW + 6),
            Err(ProofError::WrongReceipt)
        );
        assert_eq!(
            check(&offer, &record, observed, NOW + 2),
            Err(ProofError::InvalidTime)
        );
    }

    #[test]
    fn receipt_id_changes_with_offer_nonce_and_file_identity() {
        let (offer, _record, _observed) = fixture();
        let original = receipt_id_v2(&offer, 0).unwrap();
        let mut next = offer.clone();
        next.nonce[0] ^= 1;
        assert_ne!(receipt_id_v2(&next, 0).unwrap(), original);
        next = offer.clone();
        next.offer_id = Uuid::from_u128(900);
        assert_ne!(receipt_id_v2(&next, 0).unwrap(), original);
        next = offer.clone();
        next.files[0].blake3_root = *blake3::hash(b"omega").as_bytes();
        assert_ne!(receipt_id_v2(&next, 0).unwrap(), original);
        // A label-only substitution can no longer alias the receipt ID.
        next = offer.clone();
        next.files[0].label = "renamed-report.txt".into();
        assert_ne!(receipt_id_v2(&next, 0).unwrap(), original);
        next = offer.clone();
        next.receiver.device_id = DeviceId::from_uuid(Uuid::from_u128(777));
        assert_ne!(receipt_id_v2(&next, 0).unwrap(), original);
    }
    #[test]
    fn one_use_sqlite_fence_survives_cold_restart_without_retry() {
        let (offer, record, observed) = fixture();
        let checked = check(&offer, &record, observed, NOW + 6).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p2p-proof-qa.sqlite3");
        {
            let mut db = rusqlite::Connection::open(&path).unwrap();
            journal::initialize(&db).unwrap();
            journal::register_verified(&mut db, checked).unwrap();
            // Before consumption the same signed proof is idempotent.
            journal::register_verified(&mut db, checked).unwrap();
            journal::consume_once(&mut db, checked).unwrap();
            assert!(matches!(
                journal::consume_once(&mut db, checked),
                Err(journal::JournalError::AlreadyConsumed)
            ));
        }
        let mut reopened = rusqlite::Connection::open(&path).unwrap();
        journal::initialize(&reopened).unwrap();
        let checked_again = check(&offer, &record, observed, NOW + 200).unwrap();
        assert_eq!(checked_again, checked);
        assert!(matches!(
            journal::register_verified(&mut reopened, checked_again),
            Err(journal::JournalError::AlreadyConsumed)
        ));
        assert!(matches!(
            journal::consume_once(&mut reopened, checked_again),
            Err(journal::JournalError::AlreadyConsumed)
        ));
    }

    #[test]
    fn journal_refuses_unregistered_ids_and_conflicting_signed_records() {
        let (offer, record, observed) = fixture();
        let checked = check(&offer, &record, observed, NOW + 6).unwrap();
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        journal::initialize(&db).unwrap();
        assert!(matches!(
            journal::consume_once(&mut db, checked),
            Err(journal::JournalError::NotRegistered)
        ));
        journal::register_verified(&mut db, checked).unwrap();
        let mut alias = checked;
        alias.receipt_id = Uuid::from_u128(900);
        assert!(matches!(
            journal::register_verified(&mut db, alias),
            Err(journal::JournalError::Conflict)
        ));
        let mut altered = checked;
        altered.manifest_commitment[0] ^= 1;
        assert!(matches!(
            journal::register_verified(&mut db, altered),
            Err(journal::JournalError::Conflict)
        ));
        altered = checked;
        altered.proof_digest[0] ^= 1;
        assert!(matches!(
            journal::consume_once(&mut db, altered),
            Err(journal::JournalError::Conflict)
        ));
        journal::consume_once(&mut db, checked).unwrap();
    }
}
