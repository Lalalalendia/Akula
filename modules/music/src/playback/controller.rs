use super::{Effect, Error, MAX_QUEUE, MAX_UPCOMING, Phase, Repeat, Ticket, Track};
use std::num::NonZeroU64;

pub struct Player {
    pub(super) ticket: Ticket,
    pub(super) queue: Vec<Track>,
    pub(super) index: Option<usize>,
    pub(super) phase: Phase,
    pub(super) repeat: Repeat,
    pub(super) want_paused: bool,
    pub(super) position_ms: u64,
}

impl Player {
    /// The host must allocate a fresh owner for each account/controller
    /// lifetime. A new owner never accepts events from an old controller.
    pub fn new(owner: NonZeroU64) -> Self {
        Self {
            ticket: Ticket { owner, session: 0, revision: 0 },
            queue: Vec::new(),
            index: None,
            phase: Phase::Idle,
            repeat: Repeat::Off,
            want_paused: false,
            position_ms: 0,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn ticket(&self) -> Ticket {
        self.ticket
    }

    pub fn current(&self) -> Option<&Track> {
        self.index.and_then(|i| self.queue.get(i))
    }

    pub fn position_ms(&self) -> u64 {
        self.position_ms
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// The UI sees at most 30 following entries, not the entire queue.
    /// This snapshot does not wrap at the end of the list.
    pub fn upcoming(&self, limit: usize) -> &[Track] {
        let Some(i) = self.index else { return &[] };
        let start = i + 1;
        let end = (start + limit.min(MAX_UPCOMING)).min(self.queue.len());
        &self.queue[start..end]
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = repeat;
    }

    /// Replace the entire queue and start the chosen track when ready.
    /// Invalid input does not mutate state or advance the ticket.
    pub fn replace(&mut self, queue: Vec<Track>, start: usize) -> Result<Effect, Error> {
        if queue.len() > MAX_QUEUE || start >= queue.len() {
            return Err(Error::InvalidQueue);
        }
        let ticket = self.next_ticket(true)?;
        self.queue = queue;
        self.want_paused = false;
        Ok(self.install_load(start, ticket))
    }

    pub fn next_track(&mut self) -> Result<Effect, Error> {
        self.advance(false)
    }

    pub fn previous_track(&mut self) -> Result<Effect, Error> {
        let i = self.index.ok_or(Error::NoTrack)?;
        let target = if i == 0 && self.repeat == Repeat::All {
            self.queue.len() - 1
        } else {
            i.saturating_sub(1)
        };
        self.load(target)
    }

    pub fn retry(&mut self) -> Result<Effect, Error> {
        if self.phase != Phase::Failed {
            return Err(Error::InvalidState);
        }
        let i = self.index.ok_or(Error::NoTrack)?;
        self.load(i)
    }

    pub fn set_paused(&mut self, paused: bool) -> Result<Option<Effect>, Error> {
        match self.phase {
            Phase::Loading | Phase::Seeking => {
                self.want_paused = paused;
                Ok(None)
            }
            Phase::Playing | Phase::Paused => {
                if paused == self.want_paused {
                    return Ok(None);
                }
                let ticket = self.next_ticket(false)?;
                Ok(Some(self.install_pause(paused, ticket)))
            }
            _ => Err(Error::InvalidState),
        }
    }

    pub fn seek(&mut self, position_ms: u64) -> Result<Effect, Error> {
        if !matches!(self.phase, Phase::Playing | Phase::Paused | Phase::Seeking) {
            return Err(Error::InvalidState);
        }
        if position_ms > self.current().ok_or(Error::NoTrack)?.duration_ms() {
            return Err(Error::InvalidPosition);
        }
        let ticket = self.next_ticket(false)?;
        self.ticket = ticket;
        self.phase = Phase::Seeking;
        Ok(Effect::Seek { ticket, position_ms })
    }

    /// Clear the queue and invalidate every prior backend event.
    pub fn stop(&mut self) -> Result<Effect, Error> {
        let mut ticket = self.next_ticket(false)?;
        ticket.session = 0;
        self.ticket = ticket;
        self.queue.clear();
        self.index = None;
        self.phase = Phase::Idle;
        self.want_paused = false;
        self.position_ms = 0;
        Ok(Effect::Stop { ticket })
    }

    pub(super) fn next_ticket(&self, new_session: bool) -> Result<Ticket, Error> {
        let revision = self.ticket.revision.checked_add(1).ok_or(Error::RevisionExhausted)?;
        Ok(Ticket {
            owner: self.ticket.owner,
            session: if new_session { revision } else { self.ticket.session },
            revision,
        })
    }

    pub(super) fn install_pause(&mut self, paused: bool, ticket: Ticket) -> Effect {
        self.ticket = ticket;
        self.want_paused = paused;
        self.phase = if paused { Phase::Paused } else { Phase::Playing };
        Effect::SetPaused { ticket, paused }
    }

    pub(super) fn advance(&mut self, natural: bool) -> Result<Effect, Error> {
        let i = self.index.ok_or(Error::NoTrack)?;
        if natural && self.repeat == Repeat::One {
            self.load(i)
        } else if i + 1 < self.queue.len() {
            self.load(i + 1)
        } else if self.repeat == Repeat::All {
            self.load(0)
        } else {
            self.stop()
        }
    }

    fn load(&mut self, index: usize) -> Result<Effect, Error> {
        let ticket = self.next_ticket(true)?;
        Ok(self.install_load(index, ticket))
    }

    fn install_load(&mut self, index: usize, ticket: Ticket) -> Effect {
        self.ticket = ticket;
        self.index = Some(index);
        self.phase = Phase::Loading;
        self.position_ms = 0;
        Effect::Load { ticket, track: self.queue[index].clone() }
    }
}
