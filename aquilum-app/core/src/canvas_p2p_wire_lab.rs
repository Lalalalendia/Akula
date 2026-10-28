//! PC-DOC-07 offline frame preflight for future authenticated Iroh/QUIC.
//!
//! This is NOT a transport, cryptographic authentication, permission grant,
//! or a production entrypoint into Yrs. The sender argument MUST come from an
//! already authenticated QUIC endpoint, NEVER the wire, mDNS, or clipboard.
//! The signed roster is only an owner-recorded enrollment observation;
//! live identity proof, a fresh OS-backed epoch floor, and per-board editor
//! consent remain the responsibility of a future reviewed transport adapter.
//!
//! The preflight NEVER invokes a Yrs decoder, File Gate, disk, or networking.
//! Raw peer-originated Yrs bytes must not be passed to CanvasCrdtLab until
//! an upstream-safe decoder and resource budget are independently accepted.

#![deny(clippy::all)]

use crate::p2p_offer_lab::PeerBinding;
use crate::sync::device_roster_lab::{RosterStatus, VerifiedRosterObservation};
use crate::sync::{DeviceId, DocumentId, VaultId};

pub const VERSION: u8 = 1;
pub const MAX_UPDATE_BYTES: usize = 256 * 1024;
pub const HEADER_BYTES: usize = 206;
pub const MAX_FRAME_BYTES: usize = HEADER_BYTES + MAX_UPDATE_BYTES;

const MAGIC: &[u8; 8] = b"AKCANV01";
const UPDATE_KIND: u8 = 1;
const OFF_VAULT: usize = 10;
const OFF_BOARD: usize = 26;
const OFF_SESSION: usize = 42;
const OFF_EPOCH: usize = 58;
const OFF_SEQUENCE: usize = 66;
const OFF_SENDER: usize = 74;
const OFF_SENDER_ENDPOINT: usize = 90;
const OFF_RECEIVER: usize = 122;
const OFF_RECEIVER_ENDPOINT: usize = 138;
const OFF_LEN: usize = 170;
const OFF_DIGEST: usize = 174;
const DOMAIN: &str = "Akula/Canvas/offline-QUIC-frame-preflight/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    InvalidSession,
    InvalidFrame,
    UnsupportedVersion,
    TooLarge,
    WrongScope,
    WrongIdentity,
    UnknownPeer,
    Revoked,
    StaleRoster,
    NoEditorConsent,
    Corrupt,
    Replay,
    OutOfOrder,
    Exhausted,
}

/// An independently managed, *current* per-board local policy snapshot.
/// Merely appearing in the signed device roster does not imply Editor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalBoardConsent {
    Editor,
    Viewer,
    Revoked,
}

/// A local owner-policy input, NOT authority conveyed by any wire field.
/// The caller must freshly obtain this scoped decision from local consent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalBoardPolicy {
    pub vault: VaultId,
    pub board: DocumentId,
    pub sender_device: DeviceId,
    pub consent: LocalBoardConsent,
}

/// A single bound, in-memory, ordered update stream. Every new authenticated
/// QUIC session must have a fresh externally negotiated nonzero session ID.
/// A durable anti-replay high-watermark and cold-start session negotiation
/// are NOT implemented; this laboratory is never a safe shipping receiver.
pub struct OfflineFrameGate {
    vault: VaultId,
    board: DocumentId,
    sender: PeerBinding,
    receiver: PeerBinding,
    session: [u8; 16],
    roster_epoch: u64,
    next_sequence: u64,
}

impl OfflineFrameGate {
    /// Expected peers and session identity must originate outside the frame,
    /// from independently pinned local state and an authenticated QUIC channel.
    /// The roster is owner-signed, but does not itself prove channel identity.
    pub fn new(
        vault: VaultId,
        board: DocumentId,
        sender: PeerBinding,
        receiver: PeerBinding,
        session: [u8; 16],
        roster: &VerifiedRosterObservation,
    ) -> Result<Self, FrameError> {
        if vault.as_uuid().is_nil()
            || board.as_uuid().is_nil()
            || session == [0; 16]
            || sender == receiver
            || sender.device_id.as_uuid().is_nil()
            || receiver.device_id.as_uuid().is_nil()
            || sender.endpoint_id == [0; 32]
            || receiver.endpoint_id == [0; 32]
            || roster.epoch() == 0
        {
            return Err(FrameError::InvalidSession);
        }
        check_roster(roster, vault, roster.epoch(), sender, receiver)?;
        Ok(Self {
            vault,
            board,
            sender,
            receiver,
            session,
            roster_epoch: roster.epoch(),
            next_sequence: 1,
        })
    }

    pub fn expected_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Pure binary encoder for reproducible lab fixtures. The BLAKE3 digest
    /// detects accidental corruption; it is NOT a MAC or a signature. Only
    /// the already-authenticated QUIC channel could establish peer identity.
    pub fn encode_update(&self, sequence: u64, update: &[u8]) -> Result<Vec<u8>, FrameError> {
        if update.is_empty() || update.len() > MAX_UPDATE_BYTES || sequence == 0
            || sequence == u64::MAX
        {
            return Err(FrameError::TooLarge);
        }
        let mut wire = Vec::with_capacity(HEADER_BYTES + update.len());
        wire.extend_from_slice(MAGIC);
        wire.push(VERSION);
        wire.push(UPDATE_KIND);
        wire.extend_from_slice(self.vault.as_uuid().as_bytes());
        wire.extend_from_slice(self.board.as_uuid().as_bytes());
        wire.extend_from_slice(&self.session);
        wire.extend_from_slice(&self.roster_epoch.to_be_bytes());
        wire.extend_from_slice(&sequence.to_be_bytes());
        wire.extend_from_slice(self.sender.device_id.as_uuid().as_bytes());
        wire.extend_from_slice(&self.sender.endpoint_id);
        wire.extend_from_slice(self.receiver.device_id.as_uuid().as_bytes());
        wire.extend_from_slice(&self.receiver.endpoint_id);
        wire.extend_from_slice(&(update.len() as u32).to_be_bytes());
        debug_assert_eq!(wire.len(), OFF_DIGEST);
        let checksum = digest(&wire, update);
        wire.extend_from_slice(&checksum);
        wire.extend_from_slice(update);
        Ok(wire)
    }

    /// Check a single bounded frame against EXTERNAL QUIC peer identity, a
    /// fresh owner-signed roster observation, and explicit local board consent.
    /// No authority comes from the frame or its unkeyed checksum.
    ///
    /// Consumes the expected sequence ONLY on successful preflight. It does
    /// not attempt to apply the payload to Yrs, and it does not persist a
    /// replay watermark. A failed downstream CRDT apply needs a new, reviewed
    /// resync flow; do not reaccept a stale sequence.
    pub fn inspect_update<'a>(
        &mut self,
        wire: &'a [u8],
        authenticated_sender: PeerBinding,
        fresh_roster: &VerifiedRosterObservation,
        local_board_policy: LocalBoardPolicy,
    ) -> Result<&'a [u8], FrameError> {
        if wire.len() < HEADER_BYTES + 1 {
            return Err(FrameError::InvalidFrame);
        }
        if wire.len() > MAX_FRAME_BYTES {
            return Err(FrameError::TooLarge);
        }
        if authenticated_sender != self.sender {
            return Err(FrameError::WrongIdentity);
        }
        check_roster(
            fresh_roster,
            self.vault,
            self.roster_epoch,
            self.sender,
            self.receiver,
        )?;
        if local_board_policy.vault != self.vault
            || local_board_policy.board != self.board
            || local_board_policy.sender_device != self.sender.device_id
            || local_board_policy.consent != LocalBoardConsent::Editor
        {
            return Err(FrameError::NoEditorConsent);
        }
        if &wire[..8] != MAGIC || wire[8] != VERSION || wire[9] != UPDATE_KIND {
            return Err(FrameError::UnsupportedVersion);
        }
        if wire[OFF_VAULT..OFF_BOARD] != *self.vault.as_uuid().as_bytes()
            || wire[OFF_BOARD..OFF_SESSION] != *self.board.as_uuid().as_bytes()
            || wire[OFF_SESSION..OFF_EPOCH] != self.session
            || wire[OFF_EPOCH..OFF_SEQUENCE] != self.roster_epoch.to_be_bytes()
        {
            return Err(FrameError::WrongScope);
        }
        if wire[OFF_SENDER..OFF_SENDER_ENDPOINT] != *self.sender.device_id.as_uuid().as_bytes()
            || wire[OFF_SENDER_ENDPOINT..OFF_RECEIVER] != self.sender.endpoint_id
            || wire[OFF_RECEIVER..OFF_RECEIVER_ENDPOINT] != *self.receiver.device_id.as_uuid().as_bytes()
            || wire[OFF_RECEIVER_ENDPOINT..OFF_LEN] != self.receiver.endpoint_id
        {
            return Err(FrameError::WrongIdentity);
        }
        let len = u32::from_be_bytes(wire[OFF_LEN..OFF_DIGEST].try_into()
            .map_err(|_| FrameError::InvalidFrame)?) as usize;
        if len == 0 || len > MAX_UPDATE_BYTES {
            return Err(FrameError::TooLarge);
        }
        if wire.len() != HEADER_BYTES + len {
            return Err(FrameError::InvalidFrame);
        }
        let update = &wire[HEADER_BYTES..];
        if wire[OFF_DIGEST..HEADER_BYTES] != digest(&wire[..OFF_DIGEST], update) {
            return Err(FrameError::Corrupt);
        }
        let seq = u64::from_be_bytes(wire[OFF_SEQUENCE..OFF_SENDER].try_into()
            .map_err(|_| FrameError::InvalidFrame)?);
        if seq < self.next_sequence {
            return Err(FrameError::Replay);
        }
        if seq > self.next_sequence {
            return Err(FrameError::OutOfOrder);
        }
        self.next_sequence = self.next_sequence.checked_add(1)
            .ok_or(FrameError::Exhausted)?;
        Ok(update)
    }
}

fn check_roster(
    roster: &VerifiedRosterObservation,
    vault: VaultId,
    epoch: u64,
    sender: PeerBinding,
    receiver: PeerBinding,
) -> Result<(), FrameError> {
    if roster.vault_id() != vault || roster.epoch() != epoch {
        return Err(FrameError::StaleRoster);
    }
    for binding in [sender, receiver] {
        let record = roster.devices().iter()
            .find(|entry| entry.device_id() == binding.device_id)
            .ok_or(FrameError::UnknownPeer)?;
        if record.status() == RosterStatus::Revoked {
            return Err(FrameError::Revoked);
        }
        if record.endpoint_public_key() != binding.endpoint_id {
            return Err(FrameError::WrongIdentity);
        }
    }
    Ok(())
}

fn digest(header: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key(DOMAIN);
    h.update(header);
    h.update(payload);
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::replica::canvas_crdt_lab::CanvasCrdtLab;
    use crate::sync::{DeviceId, DocumentId};
    use crate::sync::device_roster_lab::{
        inspect_signed_roster, ClaimedDevice, IndependentlyPinnedOwner, OwnerSignedRoster,
    };
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use serde_json::json;
    use uuid::Uuid;
    use yrs::ClientID;

    struct Fixture {
        vault: VaultId,
        board: DocumentId,
        sender: PeerBinding,
        receiver: PeerBinding,
        roster: VerifiedRosterObservation,
    }

    fn signed_roster(
        vault: VaultId,
        sender: PeerBinding,
        receiver: PeerBinding,
        epoch: u64,
        sender_status: RosterStatus,
    ) -> VerifiedRosterObservation {
        let key = Ed25519KeyPair::from_seed_unchecked(&[11; 32]).unwrap();
        let mut claim = OwnerSignedRoster {
            vault_id: vault,
            epoch,
            devices: vec![
                ClaimedDevice {
                    device_id: sender.device_id,
                    device_public_key: [21; 32],
                    endpoint_public_key: sender.endpoint_id,
                    status: sender_status,
                },
                ClaimedDevice {
                    device_id: receiver.device_id,
                    device_public_key: [22; 32],
                    endpoint_public_key: receiver.endpoint_id,
                    status: RosterStatus::RecordedOnly,
                },
            ],
            signature: [0; 64],
        };
        claim.devices.sort_by_key(|d| d.device_id);
        let signature = key.sign(&claim.canonical_claim().unwrap());
        claim.signature.copy_from_slice(signature.as_ref());
        let pin = IndependentlyPinnedOwner {
            vault_id: vault,
            owner_public_key: key.public_key().as_ref().try_into().unwrap(),
            witnessed_min_epoch: 6,
            witnessed_epoch_digest: None,
        };
        inspect_signed_roster(&claim, &pin).unwrap()
    }

    fn fixture() -> Fixture {
        let vault = VaultId::from_uuid(Uuid::from_u128(11));
        let board = DocumentId::from_uuid(Uuid::from_u128(12));
        let sender = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(1)),
            endpoint_id: [41; 32],
        };
        let receiver = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(2)),
            endpoint_id: [42; 32],
        };
        let roster = signed_roster(vault, sender, receiver, 7, RosterStatus::RecordedOnly);
        Fixture { vault, board, sender, receiver, roster }
    }

    fn gate(f: &Fixture) -> OfflineFrameGate {
        OfflineFrameGate::new(f.vault, f.board, f.sender, f.receiver, [7; 16], &f.roster).unwrap()
    }

    fn policy(f: &Fixture, consent: LocalBoardConsent) -> LocalBoardPolicy {
        LocalBoardPolicy {
            vault: f.vault,
            board: f.board,
            sender_device: f.sender.device_id,
            consent,
        }
    }

    fn allow<'a>(
        g: &mut OfflineFrameGate,
        wire: &'a [u8],
        f: &Fixture,
    ) -> Result<&'a [u8], FrameError> {
        g.inspect_update(wire, f.sender, &f.roster, policy(f, LocalBoardConsent::Editor))
    }

    #[test]
    fn valid_binary_frame_requires_explicit_external_context() {
        let f = fixture();
        let mut g = gate(&f);
        let wire = g.encode_update(1, &[1, 2, 3]).unwrap();
        assert_eq!(wire.len(), HEADER_BYTES + 3);
        assert_eq!(allow(&mut g, &wire, &f).unwrap(), &[1, 2, 3]);
        assert_eq!(g.expected_sequence(), 2);
    }

    #[test]
    fn replay_gap_and_zero_sequence_fail_closed() {
        let f = fixture();
        let mut g = gate(&f);
        let one = g.encode_update(1, &[0, 0]).unwrap();
        assert_eq!(allow(&mut g, &one, &f).unwrap(), &[0, 0]);
        assert_eq!(allow(&mut g, &one, &f), Err(FrameError::Replay));
        let three = g.encode_update(3, &[1]).unwrap();
        assert_eq!(allow(&mut g, &three, &f), Err(FrameError::OutOfOrder));
        let zero = g.encode_update(0, &[0]);
        assert_eq!(zero, Err(FrameError::TooLarge));
        assert_eq!(g.expected_sequence(), 2);
        let two = g.encode_update(2, &[1]).unwrap();
        assert_eq!(allow(&mut g, &two, &f).unwrap(), &[1]);
    }

    #[test]
    fn cross_board_vault_and_session_frames_do_not_cross_scope() {
        let f = fixture();
        let mut g = gate(&f);
        let other_board = OfflineFrameGate::new(
            f.vault, DocumentId::new(), f.sender, f.receiver, [7; 16], &f.roster
        ).unwrap();
        assert_eq!(allow(&mut g, &other_board.encode_update(1, &[1]).unwrap(), &f),
            Err(FrameError::WrongScope));
        let other_session = OfflineFrameGate::new(
            f.vault, f.board, f.sender, f.receiver, [8; 16], &f.roster
        ).unwrap();
        assert_eq!(allow(&mut g, &other_session.encode_update(1, &[1]).unwrap(), &f),
            Err(FrameError::WrongScope));
        let different_vault = VaultId::new();
        let roster = signed_roster(different_vault, f.sender, f.receiver, 7, RosterStatus::RecordedOnly);
        let other_vault = OfflineFrameGate::new(
            different_vault, f.board, f.sender, f.receiver, [7; 16], &roster,
        ).unwrap();
        assert_eq!(allow(&mut g, &other_vault.encode_update(1, &[1]).unwrap(), &f),
            Err(FrameError::WrongScope));
        assert_eq!(g.expected_sequence(), 1);
    }

    #[test]
    fn forged_channel_peer_or_endpoint_fails_closed() {
        let f = fixture();
        let mut g = gate(&f);
        let wire = g.encode_update(1, &[1]).unwrap();
        let wrong = PeerBinding { endpoint_id: [55; 32], ..f.sender };
        assert_eq!(g.inspect_update(&wire, wrong, &f.roster, policy(&f, LocalBoardConsent::Editor)),
            Err(FrameError::WrongIdentity));
        let unknown = PeerBinding {
            device_id: DeviceId::new(), endpoint_id: [66; 32],
        };
        assert!(matches!(OfflineFrameGate::new(
            f.vault, f.board, unknown, f.receiver, [7; 16], &f.roster
        ), Err(FrameError::UnknownPeer)));
        let mut wrong_pin = f.sender;
        wrong_pin.endpoint_id = [66; 32];
        assert!(matches!(OfflineFrameGate::new(
            f.vault, f.board, wrong_pin, f.receiver, [7; 16], &f.roster
        ), Err(FrameError::WrongIdentity)));
        let mut swapped = wire.clone();
        swapped[OFF_SENDER_ENDPOINT] ^= 1;
        assert_eq!(allow(&mut g, &swapped, &f), Err(FrameError::WrongIdentity));
        assert_eq!(g.expected_sequence(), 1);
        assert_eq!(allow(&mut g, &wire, &f).unwrap(), &[1]);
    }

    #[test]
    fn revoked_and_view_only_board_consent_never_advance_sequence() {
        let f = fixture();
        let mut g = gate(&f);
        let wire = g.encode_update(1, &[1]).unwrap();
        let revoked = signed_roster(f.vault, f.sender, f.receiver, 7, RosterStatus::Revoked);
        assert_eq!(g.inspect_update(&wire, f.sender, &revoked, policy(&f, LocalBoardConsent::Editor)),
            Err(FrameError::Revoked));
        assert_eq!(g.inspect_update(&wire, f.sender, &f.roster, policy(&f, LocalBoardConsent::Viewer)),
            Err(FrameError::NoEditorConsent));
        assert_eq!(g.inspect_update(&wire, f.sender, &f.roster, policy(&f, LocalBoardConsent::Revoked)),
            Err(FrameError::NoEditorConsent));
        let wrong_vault_policy = LocalBoardPolicy {
            vault: VaultId::new(), ..policy(&f, LocalBoardConsent::Editor)
        };
        assert_eq!(g.inspect_update(&wire, f.sender, &f.roster, wrong_vault_policy),
            Err(FrameError::NoEditorConsent));
        let other_board_policy = LocalBoardPolicy {
            board: DocumentId::new(), ..policy(&f, LocalBoardConsent::Editor)
        };
        assert_eq!(g.inspect_update(&wire, f.sender, &f.roster, other_board_policy),
            Err(FrameError::NoEditorConsent));
        let other_peer_policy = LocalBoardPolicy {
            sender_device: DeviceId::new(), ..policy(&f, LocalBoardConsent::Editor)
        };
        assert_eq!(g.inspect_update(&wire, f.sender, &f.roster, other_peer_policy),
            Err(FrameError::NoEditorConsent));
        assert_eq!(g.expected_sequence(), 1);
        assert_eq!(allow(&mut g, &wire, &f).unwrap(), &[1]);
    }

    #[test]
    fn stale_roster_epoch_and_old_session_are_rejected() {
        let f = fixture();
        let mut g = gate(&f);
        let wire = g.encode_update(1, &[2]).unwrap();
        let updated_roster = signed_roster(f.vault, f.sender, f.receiver, 8, RosterStatus::RecordedOnly);
        assert_eq!(g.inspect_update(&wire, f.sender, &updated_roster, policy(&f, LocalBoardConsent::Editor)),
            Err(FrameError::StaleRoster));
        let mut new_gate = OfflineFrameGate::new(
            f.vault, f.board, f.sender, f.receiver, [9; 16], &updated_roster
        ).unwrap();
        assert_eq!(new_gate.inspect_update(&wire, f.sender, &updated_roster, policy(&f, LocalBoardConsent::Editor)),
            Err(FrameError::WrongScope));
        assert_eq!(g.expected_sequence(), 1);
    }

    #[test]
    fn altered_payload_or_checksum_does_not_advance() {
        let f = fixture();
        let mut g = gate(&f);
        let wire = g.encode_update(1, &[2, 3]).unwrap();
        let mut corrupted = wire.clone();
        corrupted[HEADER_BYTES] ^= 1;
        assert_eq!(allow(&mut g, &corrupted, &f), Err(FrameError::Corrupt));
        corrupted = wire.clone();
        corrupted[OFF_DIGEST] ^= 1;
        assert_eq!(allow(&mut g, &corrupted, &f), Err(FrameError::Corrupt));
        assert_eq!(g.expected_sequence(), 1);
        assert_eq!(allow(&mut g, &wire, &f).unwrap(), &[2, 3]);
    }

    #[test]
    fn unknown_versions_wrong_length_and_trailing_bytes_fail_closed() {
        let f = fixture();
        let mut g = gate(&f);
        let wire = g.encode_update(1, &[9]).unwrap();
        for index in [0, 8, 9] {
            let mut bad = wire.clone();
            bad[index] ^= 1;
            assert_eq!(allow(&mut g, &bad, &f), Err(FrameError::UnsupportedVersion));
        }
        let mut incorrect = wire.clone();
        incorrect[OFF_LEN + 3] = 2;
        assert_eq!(allow(&mut g, &incorrect, &f), Err(FrameError::InvalidFrame));
        let mut tail = wire.clone();
        tail.push(0);
        assert_eq!(allow(&mut g, &tail, &f), Err(FrameError::InvalidFrame));
        assert_eq!(g.expected_sequence(), 1);
    }

    #[test]
    fn frame_and_constructor_limits_precede_allocation_and_replay_tracking() {
        let f = fixture();
        let mut g = gate(&f);
        assert_eq!(allow(&mut g, &vec![0; MAX_FRAME_BYTES + 1], &f), Err(FrameError::TooLarge));
        assert_eq!(g.encode_update(1, &vec![0; MAX_UPDATE_BYTES + 1]), Err(FrameError::TooLarge));
        assert_eq!(g.encode_update(1, &[]), Err(FrameError::TooLarge));
        assert_eq!(g.encode_update(u64::MAX, &[1]), Err(FrameError::TooLarge));
        let nil = VaultId::from_uuid(Uuid::nil());
        assert!(matches!(OfflineFrameGate::new(
            nil, f.board, f.sender, f.receiver, [7; 16], &f.roster
        ), Err(FrameError::InvalidSession)));
        assert!(matches!(OfflineFrameGate::new(
            f.vault, f.board, f.sender, f.receiver, [0; 16], &f.roster
        ), Err(FrameError::InvalidSession)));
        assert_eq!(g.expected_sequence(), 1);
    }

    #[test]
    fn local_two_replica_rejoin_simulation_never_opens_network_or_files() {
        let f = fixture();
        let source = json!({
            "nodes": [{
                "id": "t", "type": "text", "text": "initial",
                "x": 0, "y": 0, "width": 240, "height": 120
            }],
            "edges": []
        });
        let a = CanvasCrdtLab::seeded(ClientID::new(201), &source).unwrap();
        let b = CanvasCrdtLab::new(ClientID::new(202));
        // Locally-created test bytes only. NOT acceptance of hostile Yrs data.
        b.apply_remote_update(&a.state()).unwrap();
        let update = a.move_node("t", 17, -23).unwrap();
        let mut receiver = gate(&f);
        let wire = receiver.encode_update(1, &update).unwrap();
        let accepted = allow(&mut receiver, &wire, &f).unwrap();
        b.apply_remote_update(accepted).unwrap();
        assert_eq!(a.snapshot().unwrap(), b.snapshot().unwrap());
        assert_eq!(b.snapshot().unwrap()["nodes"][0]["x"], 17);
        assert_eq!(receiver.expected_sequence(), 2);
    }
}
