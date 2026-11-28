//! Single-owner, deterministic queue and backend command reducer.

mod controller;
mod events;
pub mod worker;
use crate::TrackKey;
use std::num::NonZeroU64;

pub use controller::Player;

pub const MAX_QUEUE: usize = 1_000;
pub const MAX_UPCOMING: usize = 30;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Track {
    key: TrackKey,
    duration_ms: u64,
}

impl Track {
    pub fn new(key: TrackKey, duration_ms: u64) -> Result<Self, Error> {
        if duration_ms == 0 || key.provider().len() > 64 || key.id().len() > 512 {
            return Err(Error::InvalidTrack);
        }
        Ok(Self { key, duration_ms })
    }

    pub fn key(&self) -> &TrackKey {
        &self.key
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Repeat {
    Off,
    One,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Idle,
    Loading,
    Playing,
    Paused,
    Seeking,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidTrack,
    InvalidQueue,
    NoTrack,
    InvalidState,
    InvalidPosition,
    RevisionExhausted,
}

/// Owner is allocated uniquely by the host for each controller lifetime.
/// Session identifies a load/replay; revision identifies the latest command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ticket {
    owner: NonZeroU64,
    session: u64,
    revision: u64,
}

impl Ticket {
    pub fn owner(self) -> NonZeroU64 {
        self.owner
    }

    pub fn session(self) -> u64 {
        self.session
    }

    pub fn revision(self) -> u64 {
        self.revision
    }
}

/// Effects must be delivered in order by a single owner. The backend must
/// fence older tickets before starting or resuming audio. Load and Seek
/// prepare audio but keep output held until the following SetPaused effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    Load { ticket: Ticket, track: Track },
    SetPaused { ticket: Ticket, paused: bool },
    Seek { ticket: Ticket, position_ms: u64 },
    Stop { ticket: Ticket },
}

impl Effect {
    pub fn ticket(&self) -> Ticket {
        match self {
            Self::Load { ticket, .. }
            | Self::SetPaused { ticket, .. }
            | Self::Seek { ticket, .. }
            | Self::Stop { ticket } => *ticket,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    Ready,
    Progress { position_ms: u64 },
    Seeked { position_ms: u64 },
    Ended,
    Failed,
}
