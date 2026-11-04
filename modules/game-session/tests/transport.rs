use akula_game_session::{
    Command, GameSession, PeerId, Player, RoomId, Status,
    transport::{
        GameTransport, InMemoryTransport, MAX_FRAME_BYTES, MAX_QUEUED_FRAMES, TransportError,
    },
    wire::{COMMAND_BYTES, SNAPSHOT_BYTES, WIRE_VERSION, WireError, WireMessage},
};

const ROOM: RoomId = RoomId([7; 16]);
const HOST: PeerId = PeerId([1; 32]);
const GUEST: PeerId = PeerId([2; 32]);
const STRANGER: PeerId = PeerId([3; 32]);
const EPOCH: u64 = 11;

const TWENTY_TURNS: [u8; 20] = [3, 4, 6, 1, 0, 1, 0, 0, 3, 3, 3, 0, 5, 5, 5, 2, 1, 3, 2, 3];

fn command(sequence: u64, column: u8) -> Command {
    Command {
        room: ROOM,
        epoch: EPOCH,
        sequence,
        column,
    }
}

fn snapshot(game: &GameSession) -> WireMessage {
    WireMessage::Snapshot {
        room: ROOM,
        epoch: EPOCH,
        state: game.snapshot_for(HOST).unwrap().state,
    }
}

#[test]
fn command_frame_has_a_stable_exact_wire_layout() {
    let message = WireMessage::Command(command(0x0102_0304_0506_0708, 6));
    let bytes = message.encode().unwrap();
    assert_eq!(bytes.len(), COMMAND_BYTES);
    assert_eq!(&bytes[..4], &[b'A', b'G', WIRE_VERSION, 1]);
    assert_eq!(&bytes[4..20], &[7; 16]);
    assert_eq!(&bytes[20..28], &EPOCH.to_be_bytes());
    assert_eq!(&bytes[28..36], &0x0102_0304_0506_0708u64.to_be_bytes());
    assert_eq!(bytes[36], 6);
    assert_eq!(WireMessage::decode(&bytes), Ok(message));
}

#[test]
fn empty_game_snapshot_round_trips_at_fixed_length() {
    let game = GameSession::new(ROOM, EPOCH, HOST, GUEST).unwrap();
    let message = snapshot(&game);
    let bytes = message.encode().unwrap();
    assert_eq!(bytes.len(), SNAPSHOT_BYTES);
    assert_eq!(&bytes[..4], &[b'A', b'G', WIRE_VERSION, 2]);
    assert_eq!(bytes[36], 0);
    assert!(bytes[37..].iter().all(|value| *value == 0));
    assert_eq!(WireMessage::decode(&bytes), Ok(message));
}

#[test]
fn complete_wins_and_draws_have_valid_snapshots() {
    let mut game = GameSession::new(ROOM, EPOCH, HOST, GUEST).unwrap();
    let columns = [0, 6, 1, 6, 2, 5, 3];
    for (index, &col) in columns.iter().enumerate() {
        let peer = if index % 2 == 0 { HOST } else { GUEST };
        game.apply(peer, command((index / 2 + 1) as u64, col))
            .unwrap();
    }
    assert_eq!(
        game.snapshot_for(HOST).unwrap().state.status,
        Status::Won {
            winner: Player::Red
        }
    );
    let message = snapshot(&game);
    assert_eq!(WireMessage::decode(&message.encode().unwrap()), Ok(message));
}

#[test]
fn header_version_and_lengths_fail_closed() {
    let good = WireMessage::Command(command(1, 1)).encode().unwrap();
    assert_eq!(WireMessage::decode(&good[..3]), Err(WireError::TooShort));
    let mut bad = good.clone();
    bad[0] = 0;
    assert_eq!(WireMessage::decode(&bad), Err(WireError::BadMagic));
    bad = good.clone();
    bad[2] = WIRE_VERSION + 1;
    assert_eq!(
        WireMessage::decode(&bad),
        Err(WireError::UnsupportedVersion(WIRE_VERSION + 1))
    );
    bad = good.clone();
    bad[3] = 55;
    assert_eq!(
        WireMessage::decode(&bad),
        Err(WireError::UnknownMessage(55))
    );
    assert_eq!(
        WireMessage::decode(&good[..COMMAND_BYTES - 1]),
        Err(WireError::WrongLength {
            expected: COMMAND_BYTES,
            actual: COMMAND_BYTES - 1
        })
    );
    bad = good;
    bad.push(0);
    assert_eq!(
        WireMessage::decode(&bad),
        Err(WireError::WrongLength {
            expected: COMMAND_BYTES,
            actual: COMMAND_BYTES + 1
        })
    );
}

#[test]
fn invalid_room_epoch_sequence_and_column_are_refused() {
    let good = WireMessage::Command(command(1, 2)).encode().unwrap();
    for (index, want) in [
        (4, WireError::EmptyRoom),
        (27, WireError::ZeroEpoch),
        (35, WireError::ZeroSequence),
    ] {
        let mut bad = good.clone();
        if index == 4 {
            bad[4..20].fill(0);
        } else if index == 27 {
            bad[20..28].fill(0);
        } else {
            bad[28..36].fill(0);
        }
        assert_eq!(WireMessage::decode(&bad), Err(want));
    }
    let mut bad = good;
    bad[36] = 255;
    assert_eq!(WireMessage::decode(&bad), Err(WireError::ColumnOutOfRange));
}

#[test]
fn snapshots_with_corrupt_status_cells_and_revision_are_refused() {
    let game = GameSession::new(ROOM, EPOCH, HOST, GUEST).unwrap();
    let good = snapshot(&game).encode().unwrap();
    let mut bad = good.clone();
    bad[36] = 9;
    assert_eq!(WireMessage::decode(&bad), Err(WireError::InvalidStatus));
    bad = good.clone();
    bad[37] = 3;
    assert_eq!(WireMessage::decode(&bad), Err(WireError::InvalidCell));
    bad = good;
    bad[35] = 1;
    assert_eq!(WireMessage::decode(&bad), Err(WireError::InvalidState));
}

#[test]
fn gravity_parity_and_winner_checks_reject_bogus_snapshots() {
    let game = GameSession::new(ROOM, EPOCH, HOST, GUEST).unwrap();
    let good = snapshot(&game).encode().unwrap();
    let mut gravity = good.clone();
    gravity[35] = 1;
    gravity[37] = 1;
    assert_eq!(WireMessage::decode(&gravity), Err(WireError::InvalidState));
    let mut parity = good.clone();
    parity[35] = 1;
    parity[37 + 5 * 7] = 2;
    assert_eq!(WireMessage::decode(&parity), Err(WireError::InvalidState));
    let mut fake_win = good;
    fake_win[36] = 2;
    assert_eq!(WireMessage::decode(&fake_win), Err(WireError::InvalidState));
}

#[test]
fn memory_pairs_reject_fake_identity_and_destinations() {
    assert_eq!(
        InMemoryTransport::pair(HOST, HOST).unwrap_err(),
        TransportError::InvalidPeers
    );
    assert_eq!(
        InMemoryTransport::pair(HOST, PeerId([0; 32])).unwrap_err(),
        TransportError::InvalidPeers
    );
    let (host, mut guest) = InMemoryTransport::pair(HOST, GUEST).unwrap();
    assert_eq!(host.local_peer(), HOST);
    assert_eq!(guest.local_peer(), GUEST);
    assert_eq!(
        guest.send(STRANGER, b"x"),
        Err(TransportError::WrongDestination)
    );
    assert_eq!(guest.send(HOST, b""), Err(TransportError::EmptyFrame));
    assert_eq!(host.queued_frames(), Ok(0));
}

#[test]
fn frame_sender_is_bound_to_the_endpoint_not_the_payload() {
    let (mut host, mut guest) = InMemoryTransport::pair(HOST, GUEST).unwrap();
    guest.send(HOST, &[3; 32]).unwrap();
    let received = host.try_recv().unwrap().unwrap();
    assert_eq!(received.from, GUEST);
    assert_eq!(received.payload, vec![3; 32]);
    assert_eq!(host.try_recv(), Ok(None));
}

#[test]
fn in_memory_link_limits_frame_size_and_buffer_depth() {
    let (mut host, mut guest) = InMemoryTransport::pair(HOST, GUEST).unwrap();
    assert_eq!(
        guest.send(HOST, &vec![0; MAX_FRAME_BYTES + 1]),
        Err(TransportError::OversizedFrame)
    );
    assert_eq!(host.queued_frames(), Ok(0));
    for _ in 0..MAX_QUEUED_FRAMES {
        guest.send(HOST, b"x").unwrap();
    }
    assert_eq!(host.queued_frames(), Ok(MAX_QUEUED_FRAMES));
    assert_eq!(guest.send(HOST, b"x"), Err(TransportError::QueueFull));
    host.try_recv().unwrap().unwrap();
    guest.send(HOST, b"x").unwrap();
    assert_eq!(host.queued_frames(), Ok(MAX_QUEUED_FRAMES));
}

#[test]
fn closing_the_receiver_clears_pending_frames_and_prevents_reuse() {
    let (mut host, mut guest) = InMemoryTransport::pair(HOST, GUEST).unwrap();
    guest.send(HOST, b"pending").unwrap();
    host.close().unwrap();
    assert_eq!(host.queued_frames(), Ok(0));
    assert_eq!(host.try_recv(), Err(TransportError::Closed));
    assert_eq!(guest.send(HOST, b"next"), Err(TransportError::Closed));
    assert_eq!(host.send(GUEST, b"next"), Err(TransportError::Closed));
    host.close().unwrap();
}

#[test]
fn delayed_and_repeated_wire_commands_cannot_change_state_twice() {
    let (mut host, mut guest) = InMemoryTransport::pair(HOST, GUEST).unwrap();
    let mut game = GameSession::new(ROOM, EPOCH, HOST, GUEST).unwrap();
    game.apply(HOST, command(1, 1)).unwrap();
    let bytes = WireMessage::Command(command(1, 2)).encode().unwrap();
    guest.send(HOST, &bytes).unwrap();
    guest.send(HOST, &bytes).unwrap();

    for index in 0..2 {
        let received = host.try_recv().unwrap().unwrap();
        assert_eq!(received.from, GUEST);
        let WireMessage::Command(cmd) = WireMessage::decode(&received.payload).unwrap() else {
            panic!("expected command");
        };
        let result = game.apply(received.from, cmd).unwrap();
        if index == 0 {
            assert!(matches!(
                result,
                akula_game_session::ApplyResult::Applied { revision: 2, .. }
            ));
        } else {
            assert_eq!(
                result,
                akula_game_session::ApplyResult::Duplicate {
                    original_revision: 2
                }
            );
        }
    }
    assert_eq!(game.snapshot_for(HOST).unwrap().state.revision, 2);
}

#[test]
fn twenty_turns_traverse_transport_and_both_players_see_the_same_revision() {
    let (mut host, mut guest) = InMemoryTransport::pair(HOST, GUEST).unwrap();
    let mut game = GameSession::new(ROOM, EPOCH, HOST, GUEST).unwrap();
    let mut guest_state = None;
    for (index, &column) in TWENTY_TURNS.iter().enumerate() {
        let sequence = (index / 2 + 1) as u64;
        if index % 2 == 0 {
            game.apply(HOST, command(sequence, column)).unwrap();
        } else {
            let bytes = WireMessage::Command(command(sequence, column))
                .encode()
                .unwrap();
            guest.send(HOST, &bytes).unwrap();
            let packet = host.try_recv().unwrap().unwrap();
            assert_eq!(packet.from, GUEST);
            let WireMessage::Command(request) = WireMessage::decode(&packet.payload).unwrap()
            else {
                panic!("expected move");
            };
            game.apply(packet.from, request).unwrap();
        }
        let state = game.snapshot_for(HOST).unwrap().state;
        host.send(GUEST, &snapshot(&game).encode().unwrap())
            .unwrap();
        let frame = guest.try_recv().unwrap().unwrap();
        assert_eq!(frame.from, HOST);
        let WireMessage::Snapshot {
            room,
            epoch,
            state: received,
        } = WireMessage::decode(&frame.payload).unwrap()
        else {
            panic!("expected snapshot");
        };
        assert_eq!(room, ROOM);
        assert_eq!(epoch, EPOCH);
        assert_eq!(received, state);
        guest_state = Some(received);
    }
    let local = game.snapshot_for(HOST).unwrap();
    assert_eq!(guest_state.as_ref(), Some(&local.state));
    assert_eq!(local.state.revision, 20);
    assert_eq!(local.state.status, Status::InProgress { next: Player::Red });
    assert_eq!(host.queued_frames(), Ok(0));
    assert_eq!(guest.queued_frames(), Ok(0));
}
