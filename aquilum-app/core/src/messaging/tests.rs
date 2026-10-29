use super::*;

const AUTHORIZED_SIGNATURE: &[u8] = b"test-only-authorized-witness";

/// Explicitly NOT a cryptographic verifier: a deterministic test witness.
struct TestOnlyVerifier;

impl ReceiptVerifier for TestOnlyVerifier {
    fn verify(&self, candidate: &ReceiptCandidate<'_>) -> bool {
        candidate.signature == AUTHORIZED_SIGNATURE
    }
}

fn descriptor() -> Descriptor {
    Descriptor {
        id: [7; 16],
        recipient: DeviceId::new(),
        ciphertext_digest: [0xA5; 32],
        policy_epoch: 4,
        expires_at_ms: 1000,
    }
}

fn receipt<'a>(meta: &Descriptor, proof: &'a [u8]) -> ReceiptCandidate<'a> {
    ReceiptCandidate {
        id: meta.id,
        recipient: meta.recipient,
        ciphertext_digest: meta.ciphertext_digest,
        policy_epoch: meta.policy_epoch,
        signature: proof,
    }
}

fn pending() -> (Descriptor, OutboundMessage) {
    let meta = descriptor();
    let outgoing = OutboundMessage::after_durable_enqueue(meta, 0).unwrap();
    (meta, outgoing)
}

#[test]
fn no_connection_means_no_ack_until_recipient_returns() {
    let (meta, mut outgoing) = pending();
    assert_eq!(outgoing.step(10, Event::SendAttempt), Err(Error::Offline));
    assert_eq!(outgoing.phase(), Phase::LocalQueued);
    assert!(!outgoing.recipient_accepted());

    outgoing.step(20, Event::RouteAvailable).unwrap();
    assert_eq!(outgoing.step(21, Event::SendAttempt), Ok(Phase::InTransit));
    // Destination not online: no signed acknowledgement is available.
    assert_eq!(
        outgoing.step(30, Event::TransportFailed),
        Ok(Phase::NeedsRetry)
    );
    outgoing.step(40, Event::RouteLost).unwrap();
    assert!(!outgoing.recipient_accepted());

    // Recipient eventually comes online: retry with the ORIGINAL message ID.
    outgoing.step(100, Event::RouteAvailable).unwrap();
    assert_eq!(outgoing.step(101, Event::SendAttempt), Ok(Phase::InTransit));
    assert_eq!(
        outgoing.accept_receipt(
            110,
            &receipt(&meta, AUTHORIZED_SIGNATURE),
            &TestOnlyVerifier
        ),
        Ok(ReceiptOutcome::NewlyAccepted)
    );
    assert!(outgoing.recipient_accepted());
}

#[test]
fn relay_failure_is_not_recipient_acceptance() {
    let (meta, mut outgoing) = pending();
    outgoing.step(10, Event::RouteAvailable).unwrap();
    outgoing.step(11, Event::SendAttempt).unwrap();
    assert_eq!(
        outgoing.step(20, Event::RelayStored),
        Ok(Phase::RelayStored)
    );
    assert!(!outgoing.recipient_accepted());

    // The voluntary relay disappears. Its previous receipt cannot be a
    // durable acceptance receipt from the final device.
    assert_eq!(outgoing.step(30, Event::RouteLost), Ok(Phase::RelayStored));
    assert_eq!(
        outgoing.step(40, Event::TransportFailed),
        Ok(Phase::RelayStored)
    );
    assert!(!outgoing.recipient_accepted());
    outgoing.step(50, Event::RouteAvailable).unwrap();
    outgoing.step(51, Event::SendAttempt).unwrap();
    outgoing.step(60, Event::TransportFailed).unwrap();
    assert!(!outgoing.recipient_accepted());
    assert_eq!(
        outgoing.accept_receipt(70, &receipt(&meta, AUTHORIZED_SIGNATURE), &TestOnlyVerifier),
        Ok(ReceiptOutcome::NewlyAccepted)
    );
}

#[test]
fn revoked_device_cannot_later_promote_to_delivered() {
    let (meta, mut outgoing) = pending();
    outgoing.step(10, Event::RouteAvailable).unwrap();
    outgoing.step(11, Event::SendAttempt).unwrap();
    assert_eq!(
        outgoing.step(12, Event::TrustRevoked { new_epoch: 5 }),
        Ok(Phase::Revoked)
    );
    let delayed = receipt(&meta, AUTHORIZED_SIGNATURE);
    assert_eq!(
        outgoing.accept_receipt(90, &delayed, &TestOnlyVerifier),
        Err(Error::StaleEpoch)
    );
    assert_eq!(outgoing.step(100, Event::SendAttempt), Err(Error::Revoked));
    assert_eq!(
        outgoing.step(120, Event::TrustRevoked { new_epoch: 4 }),
        Err(Error::StaleEpoch)
    );
    assert_eq!(outgoing.phase(), Phase::Revoked);
    assert!(!outgoing.recipient_accepted());
}

#[test]
fn periodic_network_failures_cannot_invent_final_ack() {
    let (_meta, mut outgoing) = pending();
    for cycle in 0..5 {
        let at = 10 + cycle * 100;
        outgoing.step(at, Event::RouteAvailable).unwrap();
        outgoing.step(at + 1, Event::SendAttempt).unwrap();
        outgoing.step(at + 2, Event::RouteLost).unwrap();
        assert_eq!(outgoing.phase(), Phase::NeedsRetry);
        assert!(!outgoing.recipient_accepted());
    }
    assert_eq!(outgoing.phase(), Phase::NeedsRetry);
}

#[test]
fn forged_or_inconsistent_receipts_do_not_change_state() {
    let (meta, mut outgoing) = pending();
    outgoing.step(10, Event::RouteAvailable).unwrap();
    outgoing.step(11, Event::SendAttempt).unwrap();

    assert_eq!(
        outgoing.accept_receipt(20, &receipt(&meta, b"forged"), &TestOnlyVerifier),
        Err(Error::InvalidProof)
    );
    let mut wrong_recipient = receipt(&meta, AUTHORIZED_SIGNATURE);
    wrong_recipient.recipient = DeviceId::new();
    assert_eq!(
        outgoing.accept_receipt(21, &wrong_recipient, &TestOnlyVerifier),
        Err(Error::ReceiptMismatch)
    );
    let mut wrong_digest = receipt(&meta, AUTHORIZED_SIGNATURE);
    wrong_digest.ciphertext_digest[0] ^= 1;
    assert_eq!(
        outgoing.accept_receipt(22, &wrong_digest, &TestOnlyVerifier),
        Err(Error::ReceiptMismatch)
    );
    let mut wrong_id = receipt(&meta, AUTHORIZED_SIGNATURE);
    wrong_id.id[0] ^= 1;
    assert_eq!(
        outgoing.accept_receipt(23, &wrong_id, &TestOnlyVerifier),
        Err(Error::ReceiptMismatch)
    );
    let mut old_epoch = receipt(&meta, AUTHORIZED_SIGNATURE);
    old_epoch.policy_epoch -= 1;
    assert_eq!(
        outgoing.accept_receipt(24, &old_epoch, &TestOnlyVerifier),
        Err(Error::StaleEpoch)
    );
    assert_eq!(
        outgoing.accept_receipt(25, &receipt(&meta, &[]), &TestOnlyVerifier),
        Err(Error::InvalidProof)
    );
    assert_eq!(
        outgoing.accept_receipt(
            26,
            &receipt(&meta, &vec![1; MAX_RECEIPT_PROOF_BYTES + 1]),
            &TestOnlyVerifier
        ),
        Err(Error::InvalidProof)
    );
    assert_eq!(outgoing.phase(), Phase::InTransit);
}

#[test]
fn recipient_receipts_are_idempotent_but_must_still_be_verified() {
    let (meta, mut outgoing) = pending();
    outgoing.step(10, Event::RouteAvailable).unwrap();
    outgoing.step(11, Event::SendAttempt).unwrap();
    assert_eq!(
        outgoing.accept_receipt(15, &receipt(&meta, AUTHORIZED_SIGNATURE), &TestOnlyVerifier),
        Ok(ReceiptOutcome::NewlyAccepted)
    );
    assert_eq!(
        outgoing.accept_receipt(16, &receipt(&meta, AUTHORIZED_SIGNATURE), &TestOnlyVerifier),
        Ok(ReceiptOutcome::DuplicateAlreadyAccepted)
    );
    assert_eq!(
        outgoing.accept_receipt(17, &receipt(&meta, b"forged"), &TestOnlyVerifier),
        Err(Error::InvalidProof)
    );
    assert_eq!(outgoing.step(18, Event::SendAttempt), Err(Error::Terminal));
    assert_eq!(
        outgoing.step(19, Event::TrustRevoked { new_epoch: 5 }),
        Ok(Phase::RecipientAccepted)
    );
    assert!(outgoing.recipient_accepted());
}

#[test]
fn expiry_and_monotonic_virtual_time_fail_closed() {
    let (meta, mut outgoing) = pending();
    assert_eq!(outgoing.step(999, Event::Tick), Ok(Phase::LocalQueued));
    assert_eq!(
        outgoing.step(998, Event::RouteAvailable),
        Err(Error::TimeReversal)
    );
    assert_eq!(outgoing.now_ms(), 999);
    assert_eq!(outgoing.step(1000, Event::Tick), Ok(Phase::Expired));
    assert_eq!(outgoing.step(1001, Event::SendAttempt), Err(Error::Expired));
    assert_eq!(
        outgoing.accept_receipt(
            1002,
            &receipt(&meta, AUTHORIZED_SIGNATURE),
            &TestOnlyVerifier
        ),
        Err(Error::Expired)
    );
    assert!(!outgoing.recipient_accepted());
}

#[test]
fn unsolicited_receipt_and_relay_ack_are_rejected_before_send() {
    let (meta, mut outgoing) = pending();
    assert_eq!(
        outgoing.step(1, Event::RelayStored),
        Err(Error::InvalidTransition)
    );
    assert_eq!(
        outgoing.accept_receipt(2, &receipt(&meta, AUTHORIZED_SIGNATURE), &TestOnlyVerifier),
        Err(Error::InvalidTransition)
    );
    assert_eq!(outgoing.phase(), Phase::LocalQueued);
    assert!(!outgoing.recipient_accepted());
}

#[test]
fn malformed_descriptors_are_rejected_without_io() {
    let mut meta = descriptor();
    meta.id = [0; 16];
    assert_eq!(
        OutboundMessage::after_durable_enqueue(meta, 0).unwrap_err(),
        Error::InvalidDescriptor
    );
    meta.id = [7; 16];
    meta.policy_epoch = 0;
    assert_eq!(
        OutboundMessage::after_durable_enqueue(meta, 0).unwrap_err(),
        Error::InvalidDescriptor
    );
    meta.policy_epoch = 1;
    meta.expires_at_ms = 10;
    assert_eq!(
        OutboundMessage::after_durable_enqueue(meta, 10).unwrap_err(),
        Error::InvalidDescriptor
    );
}
