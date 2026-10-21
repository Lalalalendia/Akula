use super::*;
use crate::messaging::{Event, OutboundMessage, Phase, ReceiptCandidate, ReceiptOutcome};
use ring::signature::{Ed25519KeyPair, KeyPair};
use tempfile::tempdir;

struct Keys {
    device: DeviceId,
    epoch: u64,
    key: [u8; 32],
    active: bool,
}
impl TrustedRecipientKeys for Keys {
    fn key_for(&self, device: DeviceId, epoch: u64) -> Option<[u8; 32]> {
        if self.active && self.device == device && self.epoch == epoch {
            Some(self.key)
        } else {
            None
        }
    }
}
fn data() -> (Descriptor, Vec<u8>, Ed25519KeyPair, Keys) {
    let signer = Ed25519KeyPair::from_seed_unchecked(&[3; 32]).unwrap();
    let device = DeviceId::from_uuid(uuid::Uuid::from_bytes([1; 16]));
    let bytes = vec![0xCA, 0xB1, 0x43, 0x28, 0];
    let desc = Descriptor {
        id: [2; 16],
        recipient: device,
        ciphertext_digest: *blake3::hash(&bytes).as_bytes(),
        policy_epoch: 7,
        expires_at_ms: 9000,
    };
    let mut key = [0; 32];
    key.copy_from_slice(signer.public_key().as_ref());
    (
        desc,
        bytes,
        signer,
        Keys {
            device,
            epoch: 7,
            key,
            active: true,
        },
    )
}
fn sign(d: Descriptor, pair: &Ed25519KeyPair) -> Vec<u8> {
    let mut raw = Vec::with_capacity(RECEIPT_LEN);
    raw.push(1);
    raw.extend_from_slice(d.recipient.as_uuid().as_bytes());
    raw.extend_from_slice(&d.id);
    raw.extend_from_slice(&d.ciphertext_digest);
    raw.extend_from_slice(&d.policy_epoch.to_be_bytes());
    let mut signed = DOMAIN.to_vec();
    signed.extend_from_slice(&raw);
    raw.extend_from_slice(pair.sign(&signed).as_ref());
    assert_eq!(raw.len(), RECEIPT_LEN);
    raw
}
fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn independent_python_ed25519_receipt_vector_matches_wow_wire() {
    // Generated with cryptography Ed25519 seed 0x03*32, not the Rust signer.
    let public = hex("ed4928c628d1c2c6eae90338905995612959273a5c63f93636c14614ac8737d1");
    let raw = hex(concat!(
        "0101010101010101010101010101010101",
        "02020202020202020202020202020202",
        "a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5",
        "a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5",
        "0000000000000007",
        "5e1d0c4040f40dd9b95f2e589deedf9c8f4cc98dcc55a4e63607670d41ced8c",
        "0f34bfc7003ad91c9d725f939f40c00cffc0a4eb062d0f1251dd9ad4418312802"
    ));
    assert_eq!(raw.len(), RECEIPT_LEN);
    let (_, _, _, mut keys) = data();
    keys.key.copy_from_slice(&public);
    let verified = verify_receipt(&raw, &keys).unwrap();
    assert_eq!(verified.recipient(), keys.device);
    assert_eq!(verified.message(), [2; 16]);
    assert_eq!(verified.digest(), [0xA5; 32]);
    assert_eq!(verified.epoch(), 7);
    let mut bad = raw;
    bad[20] ^= 1;
    assert!(matches!(
        verify_receipt(&bad, &keys),
        Err(Error::InvalidSignature)
    ));
}

#[test]
fn signed_receipt_outbox_commit_and_restart_are_idempotent() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("journal.sqlite");
    let (d, bytes, signer, keys) = data();
    let proof = sign(d, &signer);
    {
        let mut j = W4Outbox::open(&path).unwrap();
        assert_eq!(j.enqueue(d, &bytes, 1).unwrap(), Insert::New);
        assert_eq!(j.enqueue(d, &bytes, 2).unwrap(), Insert::Duplicate);
        assert_eq!(
            j.retry(d.recipient, d.id, 3, &keys).unwrap(),
            Some(bytes.clone())
        );
        assert_eq!(j.status(d.recipient, d.id).unwrap(), Some(Status::Queued));
        assert_eq!(j.accept_receipt(&proof, 4, &keys).unwrap(), Accept::New);
    }
    let mut j = W4Outbox::open(&path).unwrap();
    assert_eq!(j.status(d.recipient, d.id).unwrap(), Some(Status::Accepted));
    assert_eq!(
        j.accept_receipt(&proof, 5, &keys).unwrap(),
        Accept::Duplicate
    );
    assert_eq!(j.retry(d.recipient, d.id, 6, &keys).unwrap(), None);
}

#[test]
fn wrong_signature_digest_epoch_revocation_and_replays_do_not_advance() {
    let dir = tempdir().unwrap();
    let (d, bytes, signer, mut keys) = data();
    let mut j = W4Outbox::open(&dir.path().join("deny.sqlite")).unwrap();
    j.enqueue(d, &bytes, 1).unwrap();
    let impostor = Ed25519KeyPair::from_seed_unchecked(&[4; 32]).unwrap();
    assert!(matches!(
        j.accept_receipt(&sign(d, &impostor), 2, &keys),
        Err(Error::InvalidSignature)
    ));
    let mut diff = d;
    diff.ciphertext_digest = [77; 32];
    assert!(matches!(
        j.accept_receipt(&sign(diff, &signer), 3, &keys),
        Err(Error::Mismatch)
    ));
    diff = d;
    diff.policy_epoch = 8;
    assert!(matches!(
        j.accept_receipt(&sign(diff, &signer), 4, &keys),
        Err(Error::Unauthorized)
    ));
    let raw = sign(d, &signer);
    assert!(matches!(
        j.accept_receipt(&raw[..50], 5, &keys),
        Err(Error::InvalidReceipt)
    ));
    keys.active = false;
    assert!(matches!(
        j.accept_receipt(&raw, 6, &keys),
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        j.retry(d.recipient, d.id, 7, &keys),
        Err(Error::Unauthorized)
    ));
    assert_eq!(j.status(d.recipient, d.id).unwrap(), Some(Status::Queued));
}

#[test]
fn changed_packet_same_id_conflicts_and_expiry_prevents_delivery() {
    let dir = tempdir().unwrap();
    let (d, bytes, signer, keys) = data();
    let mut j = W4Outbox::open(&dir.path().join("bad.sqlite")).unwrap();
    j.enqueue(d, &bytes, 1).unwrap();
    let changed = vec![1, 2, 3, 4, 5];
    assert!(matches!(j.enqueue(d, &changed, 2), Err(Error::Mismatch)));
    let mut new = d;
    new.ciphertext_digest = *blake3::hash(&changed).as_bytes();
    assert!(matches!(j.enqueue(new, &changed, 3), Err(Error::Conflict)));
    assert!(matches!(
        j.accept_receipt(&sign(d, &signer), 9000, &keys),
        Err(Error::Expired)
    ));
    assert_eq!(j.retry(d.recipient, d.id, 9000, &keys).unwrap(), None);
    assert_eq!(j.status(d.recipient, d.id).unwrap(), Some(Status::Queued));
}

#[test]
fn w4_verifier_advances_pure_model_only_on_true_recipient_signature() {
    let (d, _, signer, keys) = data();
    let raw = sign(d, &signer);
    let mut state = OutboundMessage::after_durable_enqueue(d, 1).unwrap();
    state.step(2, Event::RouteAvailable).unwrap();
    state.step(3, Event::SendAttempt).unwrap();
    state.step(4, Event::RelayStored).unwrap();
    assert_eq!(state.phase(), Phase::RelayStored);
    let proof = ReceiptCandidate {
        id: d.id,
        recipient: d.recipient,
        ciphertext_digest: d.ciphertext_digest,
        policy_epoch: d.policy_epoch,
        signature: &raw,
    };
    assert_eq!(
        state.accept_receipt(5, &proof, &W4Verifier { keys: &keys }),
        Ok(ReceiptOutcome::NewlyAccepted)
    );
    assert_eq!(state.phase(), Phase::RecipientAccepted);
}

#[test]
fn bounds_are_checked_before_sql_mutation() {
    let dir = tempdir().unwrap();
    let (d, _, _, _) = data();
    let mut j = W4Outbox::open(&dir.path().join("limits.sqlite")).unwrap();
    assert!(matches!(j.enqueue(d, &[], 1), Err(Error::InvalidPacket)));
    assert!(matches!(
        j.enqueue(d, &vec![0; MAX_ENVELOPE + 1], 1),
        Err(Error::InvalidPacket)
    ));
    assert!(matches!(
        j.enqueue(d, b"x", 9000),
        Err(Error::InvalidDescriptor)
    ));
    assert_eq!(j.status(d.recipient, d.id).unwrap(), None);
}
