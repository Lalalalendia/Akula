use akula_game_session::{
    ApplyResult, COLUMNS, Command, GameError, GameSession, PeerId, Player, Role, RoomId, Status,
};

const ROOM: RoomId = RoomId([9; 16]);
const HOST: PeerId = PeerId([1; 32]);
const GUEST: PeerId = PeerId([2; 32]);
const OUTSIDER: PeerId = PeerId([3; 32]);
const EPOCH: u64 = 8;

fn session() -> GameSession {
    GameSession::new(ROOM, EPOCH, HOST, GUEST).unwrap()
}

fn command(sequence: u64, column: u8) -> Command {
    Command {
        room: ROOM,
        epoch: EPOCH,
        sequence,
        column,
    }
}

fn play(session: &mut GameSession, columns: &[u8]) -> Vec<ApplyResult> {
    columns
        .iter()
        .enumerate()
        .map(|(turn, &column)| {
            let peer = if turn % 2 == 0 { HOST } else { GUEST };
            let seq = (turn / 2 + 1) as u64;
            session.apply(peer, command(seq, column)).unwrap()
        })
        .collect()
}

// A legal game: all 42 cells are filled without any win.
const DRAW_GAME: [u8; 42] = [
    3, 4, 6, 1, 0, 1, 0, 0, 3, 3, 3, 0, 5, 5, 5, 2, 1, 3, 2, 3, 4, 2, 6, 1, 4, 4, 2, 4, 6, 0, 0, 4,
    2, 6, 1, 6, 2, 5, 6, 1, 5, 5,
];

// Twenty moves do not terminate; the last move (21) forms a diagonal.
const DIAGONAL_WIN: [u8; 21] = [
    5, 0, 6, 4, 0, 4, 1, 3, 2, 4, 2, 5, 1, 5, 2, 0, 4, 3, 4, 2, 3,
];

#[test]
fn new_session_has_two_seats_and_an_empty_board() {
    let game = session();
    let host = game.snapshot_for(HOST).unwrap();
    let guest = game.snapshot_for(GUEST).unwrap();
    assert_eq!(host.room, ROOM);
    assert_eq!(host.epoch, EPOCH);
    assert_eq!(host.role, Role::Host);
    assert_eq!(host.player, Player::Red);
    assert_eq!(guest.role, Role::Guest);
    assert_eq!(guest.player, Player::Yellow);
    assert_eq!(host.next_sequence, 1);
    assert_eq!(guest.next_sequence, 1);
    assert_eq!(host.state, guest.state);
    assert_eq!(host.state.revision, 0);
    assert_eq!(host.state.status, Status::InProgress { next: Player::Red });
    assert_eq!(
        host.state
            .board
            .iter()
            .flatten()
            .filter(|p| p.is_some())
            .count(),
        0
    );
}

#[test]
fn new_session_rejects_invalid_ids_and_epoch() {
    assert_eq!(
        GameSession::new(RoomId([0; 16]), EPOCH, HOST, GUEST).unwrap_err(),
        GameError::EmptyRoomId
    );
    assert_eq!(
        GameSession::new(ROOM, EPOCH, PeerId([0; 32]), GUEST).unwrap_err(),
        GameError::EmptyPeerId
    );
    assert_eq!(
        GameSession::new(ROOM, EPOCH, HOST, PeerId([0; 32])).unwrap_err(),
        GameError::EmptyPeerId
    );
    assert_eq!(
        GameSession::new(ROOM, EPOCH, HOST, HOST).unwrap_err(),
        GameError::SamePeer
    );
    assert_eq!(
        GameSession::new(ROOM, 0, HOST, GUEST).unwrap_err(),
        GameError::ZeroEpoch
    );
}

#[test]
fn unknown_peers_cannot_play_or_read_snapshots() {
    let mut game = session();
    assert_eq!(game.snapshot_for(OUTSIDER), Err(GameError::NotParticipant));
    assert_eq!(
        game.apply(OUTSIDER, command(1, 0)),
        Err(GameError::NotParticipant)
    );
    assert_eq!(game.snapshot_for(HOST).unwrap().state.revision, 0);
}

#[test]
fn commands_are_bound_to_room_and_epoch() {
    let mut game = session();
    let before = game.snapshot_for(HOST).unwrap();
    let mut wrong = command(1, 0);
    wrong.room = RoomId([7; 16]);
    assert_eq!(game.apply(HOST, wrong), Err(GameError::WrongRoom));
    wrong.room = ROOM;
    wrong.epoch = EPOCH - 1;
    assert_eq!(
        game.apply(HOST, wrong),
        Err(GameError::WrongEpoch {
            expected: EPOCH,
            received: EPOCH - 1
        })
    );
    wrong.epoch = EPOCH + 1;
    assert_eq!(
        game.apply(HOST, wrong),
        Err(GameError::WrongEpoch {
            expected: EPOCH,
            received: EPOCH + 1
        })
    );
    assert_eq!(game.snapshot_for(HOST).unwrap(), before);
}

#[test]
fn a_previous_epochs_command_cannot_replay_in_new_room_incarnation() {
    let mut old = session();
    old.apply(HOST, command(1, 0)).unwrap();
    let mut next = GameSession::new(ROOM, EPOCH + 1, HOST, GUEST).unwrap();
    assert_eq!(
        next.apply(HOST, command(1, 0)),
        Err(GameError::WrongEpoch {
            expected: EPOCH + 1,
            received: EPOCH
        })
    );
    assert_eq!(next.snapshot_for(HOST).unwrap().state.revision, 0);
}

#[test]
fn wrong_turn_does_not_consume_sequence_or_mutate_state() {
    let mut game = session();
    let before = game.snapshot_for(GUEST).unwrap();
    assert_eq!(
        game.apply(GUEST, command(1, 0)),
        Err(GameError::NotYourTurn)
    );
    assert_eq!(game.snapshot_for(GUEST).unwrap(), before);
    game.apply(HOST, command(1, 0)).unwrap();
    assert_eq!(game.apply(HOST, command(2, 1)), Err(GameError::NotYourTurn));
    assert_eq!(game.snapshot_for(HOST).unwrap().next_sequence, 2);
    game.apply(GUEST, command(1, 1)).unwrap();
    game.apply(HOST, command(2, 2)).unwrap();
    assert_eq!(game.snapshot_for(HOST).unwrap().state.revision, 3);
}

#[test]
fn a_valid_move_lands_in_the_lowest_free_cell() {
    let mut game = session();
    assert_eq!(
        game.apply(HOST, command(1, 0)),
        Ok(ApplyResult::Applied {
            revision: 1,
            row: 5,
            column: 0,
            status: Status::InProgress {
                next: Player::Yellow
            },
        })
    );
    assert_eq!(
        game.apply(GUEST, command(1, 0)),
        Ok(ApplyResult::Applied {
            revision: 2,
            row: 4,
            column: 0,
            status: Status::InProgress { next: Player::Red },
        })
    );
    let view = game.snapshot_for(HOST).unwrap();
    assert_eq!(view.state.board[5][0], Some(Player::Red));
    assert_eq!(view.state.board[4][0], Some(Player::Yellow));
}

#[test]
fn out_of_range_column_does_not_consume_sequence() {
    let mut game = session();
    let before = game.snapshot_for(HOST).unwrap();
    assert_eq!(
        game.apply(HOST, command(1, COLUMNS as u8)),
        Err(GameError::ColumnOutOfRange)
    );
    assert_eq!(
        game.apply(HOST, command(1, u8::MAX)),
        Err(GameError::ColumnOutOfRange)
    );
    assert_eq!(game.snapshot_for(HOST).unwrap(), before);
    game.apply(HOST, command(1, 6)).unwrap();
}

#[test]
fn full_column_is_rejected_without_mutation() {
    let mut game = session();
    play(&mut game, &[0, 0, 0, 0, 0, 0]);
    let before = game.snapshot_for(HOST).unwrap();
    assert_eq!(game.apply(HOST, command(4, 0)), Err(GameError::ColumnFull));
    assert_eq!(game.snapshot_for(HOST).unwrap(), before);
    game.apply(HOST, command(4, 1)).unwrap();
    assert_eq!(game.snapshot_for(HOST).unwrap().state.revision, 7);
}

#[test]
fn the_last_accepted_command_can_be_retried_idempotently() {
    let mut game = session();
    let first = command(1, 0);
    game.apply(HOST, first).unwrap();
    let before = game.snapshot_for(HOST).unwrap();
    assert_eq!(
        game.apply(HOST, first),
        Ok(ApplyResult::Duplicate {
            original_revision: 1
        })
    );
    assert_eq!(game.snapshot_for(HOST).unwrap(), before);
    game.apply(GUEST, command(1, 6)).unwrap();
    let before = game.snapshot_for(HOST).unwrap();
    assert_eq!(
        game.apply(HOST, first),
        Ok(ApplyResult::Duplicate {
            original_revision: 1
        })
    );
    assert_eq!(game.snapshot_for(HOST).unwrap(), before);
}

#[test]
fn changing_the_column_at_same_sequence_is_rejected() {
    let mut game = session();
    game.apply(HOST, command(1, 2)).unwrap();
    let before = game.snapshot_for(HOST).unwrap();
    assert_eq!(
        game.apply(HOST, command(1, 4)),
        Err(GameError::SequenceConflict)
    );
    assert_eq!(game.snapshot_for(HOST).unwrap(), before);
}

#[test]
fn stale_and_future_sequences_are_rejected() {
    let mut game = session();
    assert_eq!(
        game.apply(HOST, command(0, 0)),
        Err(GameError::StaleSequence {
            expected: 1,
            received: 0
        })
    );
    assert_eq!(
        game.apply(HOST, command(3, 0)),
        Err(GameError::FutureSequence {
            expected: 1,
            received: 3
        })
    );
    play(&mut game, &[0, 1, 2]);
    assert_eq!(
        game.apply(HOST, command(1, 0)),
        Err(GameError::StaleSequence {
            expected: 3,
            received: 1
        })
    );
    assert_eq!(
        game.apply(HOST, command(4, 3)),
        Err(GameError::FutureSequence {
            expected: 3,
            received: 4
        })
    );
    assert_eq!(game.snapshot_for(HOST).unwrap().state.revision, 3);
}

#[test]
fn horizontal_win_stops_the_game_without_allowing_extra_moves() {
    let mut game = session();
    let results = play(&mut game, &[0, 6, 1, 6, 2, 5, 3]);
    assert_eq!(
        results.last(),
        Some(&ApplyResult::Applied {
            revision: 7,
            row: 5,
            column: 3,
            status: Status::Won {
                winner: Player::Red
            },
        })
    );
    assert_eq!(
        game.apply(GUEST, command(4, 4)),
        Err(GameError::GameFinished)
    );
    assert_eq!(game.snapshot_for(HOST).unwrap().state.revision, 7);
    assert_eq!(
        game.apply(HOST, command(4, 3)),
        Ok(ApplyResult::Duplicate {
            original_revision: 7
        })
    );
}

#[test]
fn vertical_red_win_is_detected() {
    let mut game = session();
    play(&mut game, &[0, 1, 0, 1, 0, 2, 0]);
    assert_eq!(
        game.snapshot_for(HOST).unwrap().state.status,
        Status::Won {
            winner: Player::Red
        }
    );
}

#[test]
fn vertical_yellow_win_is_detected() {
    let mut game = session();
    play(&mut game, &[1, 0, 1, 0, 2, 0, 2, 0]);
    assert_eq!(
        game.snapshot_for(GUEST).unwrap().state.status,
        Status::Won {
            winner: Player::Yellow
        }
    );
}

#[test]
fn diagonal_win_is_detected_in_both_directions() {
    let mut normal = session();
    let mut mirrored = session();
    play(&mut normal, &DIAGONAL_WIN);
    let mirror = DIAGONAL_WIN.map(|c| 6 - c);
    play(&mut mirrored, &mirror);
    assert_eq!(
        normal.snapshot_for(HOST).unwrap().state.status,
        Status::Won {
            winner: Player::Red
        }
    );
    assert_eq!(
        mirrored.snapshot_for(HOST).unwrap().state.status,
        Status::Won {
            winner: Player::Red
        }
    );
    assert_eq!(normal.snapshot_for(HOST).unwrap().state.revision, 21);
    assert_eq!(mirrored.snapshot_for(HOST).unwrap().state.revision, 21);
}

#[test]
fn twenty_turns_keep_both_players_in_the_same_state() {
    let mut game = session();
    let events = play(&mut game, &DRAW_GAME[..20]);
    assert_eq!(events.len(), 20);
    let first = game.snapshot_for(HOST).unwrap();
    let second = game.snapshot_for(GUEST).unwrap();
    assert_eq!(first.state, second.state);
    assert_eq!(first.state.revision, 20);
    assert_eq!(first.next_sequence, 11);
    assert_eq!(second.next_sequence, 11);
    assert_eq!(first.state.status, Status::InProgress { next: Player::Red });
    assert_eq!(
        first
            .state
            .board
            .iter()
            .flatten()
            .filter(|p| p.is_some())
            .count(),
        20
    );
}

#[test]
fn forty_two_legal_moves_end_in_a_draw() {
    let mut game = session();
    let events = play(&mut game, &DRAW_GAME);
    assert_eq!(events.len(), 42);
    let snapshot = game.snapshot_for(HOST).unwrap();
    assert_eq!(snapshot.state.status, Status::Draw);
    assert_eq!(snapshot.state.revision, 42);
    assert_eq!(
        snapshot
            .state
            .board
            .iter()
            .flatten()
            .filter(|p| p.is_some())
            .count(),
        42
    );
    assert_eq!(
        game.apply(HOST, command(22, 0)),
        Err(GameError::GameFinished)
    );
}

#[test]
fn every_accepted_move_increases_the_board_count_exactly_once() {
    let mut game = session();
    for (turn, &column) in DRAW_GAME.iter().enumerate() {
        let peer = if turn % 2 == 0 { HOST } else { GUEST };
        let seq = (turn / 2 + 1) as u64;
        game.apply(peer, command(seq, column)).unwrap();
        let state = game.snapshot_for(HOST).unwrap().state;
        assert_eq!(state.revision, (turn + 1) as u64);
        assert_eq!(
            state.board.iter().flatten().filter(|p| p.is_some()).count(),
            turn + 1
        );
    }
}

#[test]
fn two_fresh_sessions_replaying_the_same_inputs_produce_identical_results() {
    let mut a = session();
    let mut b = session();
    let events_a = play(&mut a, &DRAW_GAME[..20]);
    let events_b = play(&mut b, &DRAW_GAME[..20]);
    assert_eq!(events_a, events_b);
    assert_eq!(a.snapshot_for(HOST), b.snapshot_for(HOST));
    assert_eq!(a.snapshot_for(GUEST), b.snapshot_for(GUEST));
}

#[test]
fn a_move_rejected_for_turn_order_can_be_retried_when_it_becomes_legal() {
    let mut game = session();
    let early = command(1, 4);
    assert_eq!(game.apply(GUEST, early), Err(GameError::NotYourTurn));
    game.apply(HOST, command(1, 3)).unwrap();
    assert!(matches!(
        game.apply(GUEST, early),
        Ok(ApplyResult::Applied { revision: 2, .. })
    ));
}

#[test]
fn previous_game_command_cannot_be_accepted_by_another_room() {
    let mut game = GameSession::new(RoomId([4; 16]), EPOCH, HOST, GUEST).unwrap();
    assert_eq!(game.apply(HOST, command(1, 2)), Err(GameError::WrongRoom));
    assert_eq!(game.snapshot_for(HOST).unwrap().state.revision, 0);
}
