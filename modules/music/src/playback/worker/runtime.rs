use std::num::NonZeroU64;

use crate::playback::{Effect, Error, Event, Phase, Player, Ticket};

use super::{BackendError, Command, Mailbox, MockBackend, SubmitError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    Controller(Error),
    Backend(BackendError),
    Poisoned,
    OwnerReuse,
}

/// Single owner drives the reducer and the backend together. No effect is
/// queued after the reducer changes; a failed handoff holds output and
/// poisons this lifetime instead of continuing with divergent state.
pub struct Runtime {
    owner: NonZeroU64,
    mailbox: Mailbox,
    player: Player,
    backend: MockBackend,
    poisoned: bool,
}

impl Runtime {
    pub fn new(owner: NonZeroU64) -> Self {
        Self {
            owner,
            mailbox: Mailbox::new(owner),
            player: Player::new(owner),
            backend: MockBackend::new(owner),
            poisoned: false,
        }
    }

    pub fn player(&self) -> &Player {
        &self.player
    }

    pub fn backend(&self) -> &MockBackend {
        &self.backend
    }

    pub fn queued(&self) -> usize {
        self.mailbox.len()
    }

    pub fn submit(&mut self, owner: NonZeroU64, command: Command) -> Result<(), (SubmitError, Command)> {
        self.mailbox.submit(owner, command)
    }

    pub fn step(&mut self) -> Result<bool, RuntimeError> {
        self.check_live()?;
        let Some(command) = self.mailbox.pop() else { return Ok(false) };
        let effect = match command {
            Command::Replace { tracks, start } => self.player.replace(tracks, start).map(Some),
            Command::Next => self.player.next_track().map(Some),
            Command::Previous => self.player.previous_track().map(Some),
            Command::Pause(paused) => self.player.set_paused(paused),
            Command::Seek(position) => self.player.seek(position).map(Some),
            Command::Retry => self.player.retry().map(Some),
            Command::Repeat(repeat) => {
                self.player.set_repeat(repeat);
                Ok(None)
            }
            Command::Stop => self.player.stop().map(Some),
        }.map_err(RuntimeError::Controller)?;
        self.dispatch(effect)?;
        Ok(true)
    }

    pub fn complete(&mut self, ticket: Ticket, success: bool) -> Result<bool, RuntimeError> {
        self.check_live()?;
        let event = self.backend.complete(ticket, success);
        self.receive(ticket, event)
    }

    pub fn notify(&mut self, ticket: Ticket, event: Event) -> Result<bool, RuntimeError> {
        self.check_live()?;
        let event = self.backend.notify(ticket, event);
        self.receive(ticket, event)
    }

    /// The host must allocate a globally fresh owner; this check prevents
    /// reusing the immediately active owner, not historic IDs across process runs.
    pub fn switch_owner(&mut self, owner: NonZeroU64) -> Result<(), RuntimeError> {
        if owner == self.owner {
            return Err(RuntimeError::OwnerReuse);
        }
        self.backend.quiesce();
        *self = Self::new(owner);
        Ok(())
    }

    fn check_live(&self) -> Result<(), RuntimeError> {
        if self.poisoned { Err(RuntimeError::Poisoned) } else { Ok(()) }
    }

    fn receive(&mut self, ticket: Ticket, event: Option<Event>) -> Result<bool, RuntimeError> {
        let Some(event) = event else { return Ok(false) };
        // If a terminal notification has already held output, a reducer
        // error must not permit a divergent playing state to continue.
        let effect = match self.player.receive(ticket, event) {
            Ok(effect) => effect,
            Err(error) => {
                self.backend.quiesce();
                self.poisoned = true;
                return Err(RuntimeError::Controller(error));
            }
        };
        self.dispatch(effect)?;
        Ok(true)
    }

    fn dispatch(&mut self, effect: Option<Effect>) -> Result<(), RuntimeError> {
        if let Some(effect) = effect {
            if let Err(error) = self.backend.accept(effect) {
                self.backend.quiesce();
                self.poisoned = true;
                return Err(RuntimeError::Backend(error));
            }
        }
        debug_assert!(!self.backend.playing() || self.player.phase() == Phase::Playing);
        Ok(())
    }
}
