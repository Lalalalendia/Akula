use std::num::NonZeroU64;

use crate::playback::{Effect, Event, Ticket, Track};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendError {
    WrongOwner,
    StaleRevision,
    WrongSession,
    InvalidState,
    InvalidPosition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pending {
    Load,
    Seek(u64),
}

/// Inert audio-device model with at most one outstanding load/seek.
/// Command acceptance fences previous operations before any completion.
pub struct MockBackend {
    owner: NonZeroU64,
    current: Option<Ticket>,
    track: Option<Track>,
    pending: Option<Pending>,
    prepared: bool,
    playing: bool,
}

impl MockBackend {
    pub fn new(owner: NonZeroU64) -> Self {
        Self {
            owner,
            current: None,
            track: None,
            pending: None,
            prepared: false,
            playing: false,
        }
    }

    pub fn ticket(&self) -> Option<Ticket> {
        self.current
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn track(&self) -> Option<&Track> {
        self.track.as_ref()
    }

    /// Reject malformed or stale commands without modifying current output.
    pub fn accept(&mut self, effect: Effect) -> Result<(), BackendError> {
        let ticket = effect.ticket();
        if ticket.owner() != self.owner {
            return Err(BackendError::WrongOwner);
        }
        if self.current.is_some_and(|t| ticket.revision() <= t.revision()) {
            return Err(BackendError::StaleRevision);
        }
        match effect {
            Effect::Load { track, .. } => {
                if ticket.session() == 0 || ticket.session() != ticket.revision() {
                    return Err(BackendError::WrongSession);
                }
                self.quiesce();
                self.track = Some(track);
                self.pending = Some(Pending::Load);
            }
            Effect::Seek { position_ms, .. } => {
                self.require_session(ticket)?;
                if !self.prepared && !matches!(self.pending, Some(Pending::Seek(_))) {
                    return Err(BackendError::InvalidState);
                }
                if position_ms > self.track.as_ref().ok_or(BackendError::InvalidState)?.duration_ms() {
                    return Err(BackendError::InvalidPosition);
                }
                self.playing = false;
                self.prepared = false;
                self.pending = Some(Pending::Seek(position_ms));
            }
            Effect::SetPaused { paused, .. } => {
                self.require_session(ticket)?;
                if !self.prepared || self.pending.is_some() {
                    return Err(BackendError::InvalidState);
                }
                self.playing = !paused;
            }
            Effect::Stop { .. } => {
                if ticket.session() != 0 {
                    self.require_session(ticket)?;
                }
                self.quiesce();
            }
        }
        self.current = Some(ticket);
        Ok(())
    }

    fn require_session(&self, ticket: Ticket) -> Result<(), BackendError> {
        if ticket.session() == 0 || self.current.map(Ticket::session) != Some(ticket.session()) {
            return Err(BackendError::WrongSession);
        }
        Ok(())
    }

    pub(super) fn quiesce(&mut self) {
        self.track = None;
        self.pending = None;
        self.prepared = false;
        self.playing = false;
    }

    /// Completion is consumed once. Even a current successful load/seek
    /// leaves output held until the controller issues SetPaused.
    pub fn complete(&mut self, ticket: Ticket, success: bool) -> Option<Event> {
        if self.current != Some(ticket) {
            return None;
        }
        let pending = self.pending.take()?;
        if !success {
            self.quiesce();
            return Some(Event::Failed);
        }
        self.prepared = true;
        Some(match pending {
            Pending::Load => Event::Ready,
            Pending::Seek(position_ms) => Event::Seeked { position_ms },
        })
    }

    /// Model a device notification, not real rendered audio.
    pub fn notify(&mut self, ticket: Ticket, event: Event) -> Option<Event> {
        if self.current != Some(ticket) || !self.playing {
            return None;
        }
        match event {
            Event::Progress { .. } => Some(event),
            Event::Ended | Event::Failed => {
                self.playing = false;
                Some(event)
            }
            _ => None,
        }
    }
}
