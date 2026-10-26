//! This is a state-contract *scale smoke test*, NOT a physical radio simulation.
//! 2,000 installed-Akula logical clients share no production network or database.

use super::{
    Descriptor, Error, Event, OutboundMessage, Phase, ReceiptCandidate, ReceiptOutcome,
    ReceiptVerifier,
};
use crate::sync::DeviceId;
use uuid::Uuid;

struct FakeScaleReceiptVerifier;

impl ReceiptVerifier for FakeScaleReceiptVerifier {
    fn verify(&self, receipt: &ReceiptCandidate<'_>) -> bool {
        receipt.signature == b"test-scale-only"
    }
}

#[test]
fn two_thousand_akula_devices_never_upgrade_relay_or_revocation_to_delivery() {
    let mut floor_counts = [0usize; 5];
    let mut room_counts = [[0usize; 70]; 5];
    let mut accepted = 0usize;
    let mut revoked = 0usize;

    for index in 0..2_000u128 {
        // Five floors, 70 rooms per floor and exactly 400 clients per floor.
        // Layout is a fixture; this is not an RF/contact graph measurement.
        let floor = (index as usize) / 400;
        let room = (index as usize % 400) % 70;
        floor_counts[floor] += 1;
        room_counts[floor][room] += 1;

        let descriptor = Descriptor {
            id: (index + 1).to_be_bytes(),
            recipient: DeviceId::from_uuid(Uuid::from_u128(index + 1)),
            ciphertext_digest: [0xA5; 32],
            policy_epoch: 1,
            expires_at_ms: 1_000,
        };
        let mut state = OutboundMessage::after_durable_enqueue(descriptor, 0).unwrap();
        match index % 4 {
            0 => {
                // Recipient unreachable before first send.
                assert_eq!(state.step(10, Event::SendAttempt), Err(Error::Offline));
            }
            1 => {
                state.step(10, Event::RouteAvailable).unwrap();
                state.step(11, Event::SendAttempt).unwrap();
                state.step(12, Event::RelayStored).unwrap();
                state.step(13, Event::RouteLost).unwrap();
                assert_eq!(state.phase(), Phase::RelayStored);
            }
            2 => {
                state.step(10, Event::RouteAvailable).unwrap();
                state.step(11, Event::SendAttempt).unwrap();
                state.step(12, Event::TransportFailed).unwrap();
                assert_eq!(state.phase(), Phase::NeedsRetry);
            }
            3 => {
                state.step(10, Event::RouteAvailable).unwrap();
                state.step(11, Event::SendAttempt).unwrap();
                state
                    .step(12, Event::TrustRevoked { new_epoch: 2 })
                    .unwrap();
                assert_eq!(state.phase(), Phase::Revoked);
            }
            _ => unreachable!(),
        }
        assert!(!state.recipient_accepted());

        let candidate = ReceiptCandidate {
            id: descriptor.id,
            recipient: descriptor.recipient,
            ciphertext_digest: descriptor.ciphertext_digest,
            policy_epoch: descriptor.policy_epoch,
            signature: b"test-scale-only",
        };
        if index % 4 == 3 {
            assert_eq!(
                state.accept_receipt(110, &candidate, &FakeScaleReceiptVerifier),
                Err(Error::StaleEpoch)
            );
            revoked += 1;
            continue;
        }
        state.step(100, Event::RouteAvailable).unwrap();
        state.step(101, Event::SendAttempt).unwrap();
        assert_eq!(
            state.accept_receipt(110, &candidate, &FakeScaleReceiptVerifier),
            Ok(ReceiptOutcome::NewlyAccepted)
        );
        assert!(state.recipient_accepted());
        accepted += 1;
    }
    assert_eq!(floor_counts, [400; 5]);
    for floor in room_counts {
        assert!(floor.iter().all(|count| (5..=6).contains(count)));
    }
    assert_eq!(accepted, 1_500);
    assert_eq!(revoked, 500);
}
