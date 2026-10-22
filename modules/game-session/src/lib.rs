//! GNET-01: a deterministic, transport-independent Connect Four game session.
//!
//! The caller MUST pass the identity authenticated by its transport to apply.
//! Neither a claimed identity in a packet nor a room ID is authentication.
//! Room IDs and epochs must be issued and persisted by the future room owner.
//! This crate performs no I/O, opens no sockets, and has no file or Akula access.

pub const ROWS: usize = 6;
pub const COLUMNS: usize = 7;
pub type Board = [[Option<Player>; COLUMNS]; ROWS];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RoomId(pub [u8; 16]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PeerId(pub [u8; 32]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Player {
    Red,
    Yellow,
}

impl Player {
    fn opponent(self) -> Self {
        match self {
            Self::Red => Self::Yellow,
            Self::Yellow => Self::Red,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Host,
    Guest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    InProgress { next: Player },
    Won { winner: Player },
    Draw,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameState {
    /// Index 0 is the top row; index ROWS - 1 is the bottom.
    pub board: Board,
    pub revision: u64,
    pub status: Status,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerView {
    pub room: RoomId,
    pub epoch: u64,
    pub role: Role,
    pub player: Player,
    /// The next accepted command must use this sequence number.
    pub next_sequence: u64,
    pub state: GameState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    pub room: RoomId,
    pub epoch: u64,
    /// Strictly increasing per authenticated peer, starting at 1.
    pub sequence: u64,
    pub column: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyResult {
    Applied {
        revision: u64,
        row: u8,
        column: u8,
        status: Status,
    },
    /// Exactly the last accepted command was repeated; no state was changed.
    Duplicate { original_revision: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameError {
    EmptyRoomId,
    EmptyPeerId,
    SamePeer,
    ZeroEpoch,
    NotParticipant,
    WrongRoom,
    WrongEpoch { expected: u64, received: u64 },
    StaleSequence { expected: u64, received: u64 },
    FutureSequence { expected: u64, received: u64 },
    SequenceConflict,
    NotYourTurn,
    ColumnOutOfRange,
    ColumnFull,
    GameFinished,
}

#[derive(Clone, Copy, Debug)]
struct AcceptedCommand {
    sequence: u64,
    column: u8,
    revision: u64,
}

/// Host-authoritative session with two admitted participants.
/// Network join, authentication, secret handling and persistence are OUT OF SCOPE.
#[derive(Clone, Debug)]
pub struct GameSession {
    room: RoomId,
    epoch: u64,
    peers: [PeerId; 2],
    next_sequences: [u64; 2],
    last_accepted: [Option<AcceptedCommand>; 2],
    state: GameState,
}

impl GameSession {
    /// Host always plays Red; the invited guest plays Yellow in GNET-01.
    pub fn new(room: RoomId, epoch: u64, host: PeerId, guest: PeerId) -> Result<Self, GameError> {
        if room.0 == [0; 16] {
            return Err(GameError::EmptyRoomId);
        }
        if host.0 == [0; 32] || guest.0 == [0; 32] {
            return Err(GameError::EmptyPeerId);
        }
        if host == guest {
            return Err(GameError::SamePeer);
        }
        if epoch == 0 {
            return Err(GameError::ZeroEpoch);
        }
        Ok(Self {
            room,
            epoch,
            peers: [host, guest],
            next_sequences: [1, 1],
            last_accepted: [None, None],
            state: GameState {
                board: [[None; COLUMNS]; ROWS],
                revision: 0,
                status: Status::InProgress { next: Player::Red },
            },
        })
    }

    /// Returns only the state view of an admitted authenticated participant.
    pub fn snapshot_for(&self, peer: PeerId) -> Result<PlayerView, GameError> {
        let index = self.peer_index(peer)?;
        Ok(PlayerView {
            room: self.room,
            epoch: self.epoch,
            role: if index == 0 { Role::Host } else { Role::Guest },
            player: Self::player(index),
            next_sequence: self.next_sequences[index],
            state: self.state.clone(),
        })
    }

    /// Apply a command on the authoritative host.
    /// The peer MUST come from the authenticated connection, never from the payload.
    pub fn apply(&mut self, peer: PeerId, command: Command) -> Result<ApplyResult, GameError> {
        // Check the sender first, before revealing room or sequence state.
        let index = self.peer_index(peer)?;
        if command.room != self.room {
            return Err(GameError::WrongRoom);
        }
        if command.epoch != self.epoch {
            return Err(GameError::WrongEpoch {
                expected: self.epoch,
                received: command.epoch,
            });
        }

        if let Some(last) = self.last_accepted[index]
            && command.sequence == last.sequence
        {
            return if command.column == last.column {
                Ok(ApplyResult::Duplicate {
                    original_revision: last.revision,
                })
            } else {
                Err(GameError::SequenceConflict)
            };
        }
        let expected = self.next_sequences[index];
        if command.sequence < expected {
            return Err(GameError::StaleSequence {
                expected,
                received: command.sequence,
            });
        }
        if command.sequence > expected {
            return Err(GameError::FutureSequence {
                expected,
                received: command.sequence,
            });
        }

        let mover = Self::player(index);
        match self.state.status {
            Status::InProgress { next } if next == mover => {}
            Status::InProgress { .. } => return Err(GameError::NotYourTurn),
            Status::Won { .. } | Status::Draw => return Err(GameError::GameFinished),
        }
        let col = usize::from(command.column);
        if col >= COLUMNS {
            return Err(GameError::ColumnOutOfRange);
        }
        let row = (0..ROWS)
            .rev()
            .find(|&r| self.state.board[r][col].is_none())
            .ok_or(GameError::ColumnFull)?;

        self.state.board[row][col] = Some(mover);
        self.state.revision += 1;
        self.state.status = if self.is_winning_move(row, col, mover) {
            Status::Won { winner: mover }
        } else if self.state.revision == (ROWS * COLUMNS) as u64 {
            Status::Draw
        } else {
            Status::InProgress {
                next: mover.opponent(),
            }
        };
        let revision = self.state.revision;
        self.last_accepted[index] = Some(AcceptedCommand {
            sequence: command.sequence,
            column: command.column,
            revision,
        });
        // At most 42 valid turns in this game. No sequence counter can overflow.
        self.next_sequences[index] += 1;
        Ok(ApplyResult::Applied {
            revision,
            row: row as u8,
            column: command.column,
            status: self.state.status,
        })
    }

    fn peer_index(&self, peer: PeerId) -> Result<usize, GameError> {
        if peer == self.peers[0] {
            Ok(0)
        } else if peer == self.peers[1] {
            Ok(1)
        } else {
            Err(GameError::NotParticipant)
        }
    }

    fn player(index: usize) -> Player {
        if index == 0 {
            Player::Red
        } else {
            Player::Yellow
        }
    }

    fn is_winning_move(&self, row: usize, col: usize, player: Player) -> bool {
        [(1, 0), (0, 1), (1, 1), (1, -1)]
            .into_iter()
            .any(|(dr, dc)| {
                1 + self.count_direction(row, col, dr, dc, player)
                    + self.count_direction(row, col, -dr, -dc, player)
                    >= 4
            })
    }

    fn count_direction(&self, row: usize, col: usize, dr: i32, dc: i32, player: Player) -> u8 {
        let mut r = row as i32 + dr;
        let mut c = col as i32 + dc;
        let mut count = 0;
        while r >= 0
            && r < ROWS as i32
            && c >= 0
            && c < COLUMNS as i32
            && self.state.board[r as usize][c as usize] == Some(player)
        {
            count += 1;
            r += dr;
            c += dc;
        }
        count
    }
}
