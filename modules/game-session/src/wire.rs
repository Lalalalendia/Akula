//! GNET-02: bounded, deterministic network representation.
//!
//! This module has no socket dependency. Transport authentication, joining a
//! room and checking whether a sender may publish snapshots are caller duties.

use crate::{Board, COLUMNS, Command, GameState, Player, ROWS, RoomId, Status};

pub const WIRE_VERSION: u8 = 1;
pub const COMMAND_BYTES: usize = 37;
pub const SNAPSHOT_BYTES: usize = 79;

const MAGIC: &[u8; 2] = b"AG";
const COMMAND_TYPE: u8 = 1;
const SNAPSHOT_TYPE: u8 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireMessage {
    Command(Command),
    Snapshot {
        room: RoomId,
        epoch: u64,
        state: GameState,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    TooShort,
    BadMagic,
    UnsupportedVersion(u8),
    UnknownMessage(u8),
    WrongLength { expected: usize, actual: usize },
    EmptyRoom,
    ZeroEpoch,
    ZeroSequence,
    ColumnOutOfRange,
    InvalidCell,
    InvalidStatus,
    InvalidState,
}

impl WireMessage {
    /// Serialize using fixed-length v1 frames and network byte order.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        match self {
            Self::Command(command) => {
                check_room_epoch(command.room, command.epoch)?;
                if command.sequence == 0 {
                    return Err(WireError::ZeroSequence);
                }
                if usize::from(command.column) >= COLUMNS {
                    return Err(WireError::ColumnOutOfRange);
                }
                let mut data = Vec::with_capacity(COMMAND_BYTES);
                push_prefix(&mut data, COMMAND_TYPE, command.room, command.epoch);
                data.extend_from_slice(&command.sequence.to_be_bytes());
                data.push(command.column);
                Ok(data)
            }
            Self::Snapshot { room, epoch, state } => {
                check_room_epoch(*room, *epoch)?;
                validate_state(state)?;
                let mut data = Vec::with_capacity(SNAPSHOT_BYTES);
                push_prefix(&mut data, SNAPSHOT_TYPE, *room, *epoch);
                data.extend_from_slice(&state.revision.to_be_bytes());
                data.push(status_byte(state.status));
                for cell in state.board.iter().flatten() {
                    data.push(match cell {
                        None => 0,
                        Some(Player::Red) => 1,
                        Some(Player::Yellow) => 2,
                    });
                }
                Ok(data)
            }
        }
    }

    /// Strictly parse one whole frame, rejecting extra bytes and malformed state.
    pub fn decode(data: &[u8]) -> Result<Self, WireError> {
        if data.len() < 4 {
            return Err(WireError::TooShort);
        }
        if &data[..2] != MAGIC {
            return Err(WireError::BadMagic);
        }
        if data[2] != WIRE_VERSION {
            return Err(WireError::UnsupportedVersion(data[2]));
        }
        let expected = match data[3] {
            COMMAND_TYPE => COMMAND_BYTES,
            SNAPSHOT_TYPE => SNAPSHOT_BYTES,
            other => return Err(WireError::UnknownMessage(other)),
        };
        if data.len() != expected {
            return Err(WireError::WrongLength {
                expected,
                actual: data.len(),
            });
        }
        let mut id = [0u8; 16];
        id.copy_from_slice(&data[4..20]);
        let room = RoomId(id);
        let epoch = read_u64(&data[20..28]);
        check_room_epoch(room, epoch)?;

        match data[3] {
            COMMAND_TYPE => {
                let sequence = read_u64(&data[28..36]);
                if sequence == 0 {
                    return Err(WireError::ZeroSequence);
                }
                let column = data[36];
                if usize::from(column) >= COLUMNS {
                    return Err(WireError::ColumnOutOfRange);
                }
                Ok(Self::Command(Command {
                    room,
                    epoch,
                    sequence,
                    column,
                }))
            }
            SNAPSHOT_TYPE => {
                let revision = read_u64(&data[28..36]);
                let status = read_status(data[36])?;
                let mut board: Board = [[None; COLUMNS]; ROWS];
                for (cell, byte) in board.iter_mut().flatten().zip(&data[37..]) {
                    *cell = match byte {
                        0 => None,
                        1 => Some(Player::Red),
                        2 => Some(Player::Yellow),
                        _ => return Err(WireError::InvalidCell),
                    };
                }
                let state = GameState {
                    board,
                    revision,
                    status,
                };
                validate_state(&state)?;
                Ok(Self::Snapshot { room, epoch, state })
            }
            _ => unreachable!("type checked above"),
        }
    }
}

fn check_room_epoch(room: RoomId, epoch: u64) -> Result<(), WireError> {
    if room.0 == [0; 16] {
        return Err(WireError::EmptyRoom);
    }
    if epoch == 0 {
        return Err(WireError::ZeroEpoch);
    }
    Ok(())
}

fn push_prefix(data: &mut Vec<u8>, kind: u8, room: RoomId, epoch: u64) {
    data.extend_from_slice(MAGIC);
    data.push(WIRE_VERSION);
    data.push(kind);
    data.extend_from_slice(&room.0);
    data.extend_from_slice(&epoch.to_be_bytes());
}

fn read_u64(data: &[u8]) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(data);
    u64::from_be_bytes(bytes)
}

fn status_byte(status: Status) -> u8 {
    match status {
        Status::InProgress { next: Player::Red } => 0,
        Status::InProgress {
            next: Player::Yellow,
        } => 1,
        Status::Won {
            winner: Player::Red,
        } => 2,
        Status::Won {
            winner: Player::Yellow,
        } => 3,
        Status::Draw => 4,
    }
}

fn read_status(byte: u8) -> Result<Status, WireError> {
    match byte {
        0 => Ok(Status::InProgress { next: Player::Red }),
        1 => Ok(Status::InProgress {
            next: Player::Yellow,
        }),
        2 => Ok(Status::Won {
            winner: Player::Red,
        }),
        3 => Ok(Status::Won {
            winner: Player::Yellow,
        }),
        4 => Ok(Status::Draw),
        _ => Err(WireError::InvalidStatus),
    }
}

/// Validate the public state invariants before accepting or transmitting it.
fn validate_state(state: &GameState) -> Result<(), WireError> {
    let mut red = 0usize;
    let mut yellow = 0usize;
    for row in &state.board {
        for cell in row {
            match cell {
                Some(Player::Red) => red += 1,
                Some(Player::Yellow) => yellow += 1,
                None => {}
            }
        }
    }
    if state.revision != (red + yellow) as u64 || red < yellow || red > yellow + 1 {
        return Err(WireError::InvalidState);
    }
    for col in 0..COLUMNS {
        let mut seen_empty_below = false;
        for row in (0..ROWS).rev() {
            if state.board[row][col].is_some() {
                if seen_empty_below {
                    return Err(WireError::InvalidState);
                }
            } else {
                seen_empty_below = true;
            }
        }
    }
    let red_wins = has_four(&state.board, Player::Red);
    let yellow_wins = has_four(&state.board, Player::Yellow);
    let legal = match state.status {
        Status::InProgress { next: Player::Red } => {
            red == yellow && !red_wins && !yellow_wins && state.revision < 42
        }
        Status::InProgress {
            next: Player::Yellow,
        } => red == yellow + 1 && !red_wins && !yellow_wins && state.revision < 42,
        Status::Won {
            winner: Player::Red,
        } => red == yellow + 1 && red_wins && !yellow_wins,
        Status::Won {
            winner: Player::Yellow,
        } => red == yellow && yellow_wins && !red_wins,
        Status::Draw => state.revision == 42 && red == yellow && !red_wins && !yellow_wins,
    };
    if !legal {
        return Err(WireError::InvalidState);
    }
    Ok(())
}

fn has_four(board: &Board, player: Player) -> bool {
    for row in 0..ROWS {
        for col in 0..COLUMNS {
            if board[row][col] != Some(player) {
                continue;
            }
            for (dr, dc) in [(1, 0), (0, 1), (1, 1), (1, -1)] {
                let found = (1..4).all(|step| {
                    let r = row as i32 + dr * step;
                    let c = col as i32 + dc * step;
                    r >= 0
                        && r < ROWS as i32
                        && c >= 0
                        && c < COLUMNS as i32
                        && board[r as usize][c as usize] == Some(player)
                });
                if found {
                    return true;
                }
            }
        }
    }
    false
}
