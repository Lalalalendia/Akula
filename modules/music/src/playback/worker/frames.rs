//! Account for output frames, not interleaved channel samples or media position.
//! Reports are cumulative per session, from an external trusted output backend.

use crate::playback::Ticket;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    InvalidRate,
    InvalidSession,
    InvalidRevision,
    CounterRegressed,
    RenderedWhileHeld,
}

pub struct RenderedClock {
    ticket: Ticket,
    sample_rate: u32,
    frames: u64,
    audible: bool,
}

impl RenderedClock {
    /// Start held. Only a fresh load ticket starts an accounting session.
    /// Sample rate is fixed for its lifetime; format change requires a new
    /// session or a future explicit rate-segment contract.
    pub fn new(load: Ticket, sample_rate: u32) -> Result<Self, FrameError> {
        if !(8_000..=384_000).contains(&sample_rate) {
            return Err(FrameError::InvalidRate);
        }
        if load.session() == 0 || load.session() != load.revision() {
            return Err(FrameError::InvalidSession);
        }
        Ok(Self {
            ticket: load,
            sample_rate,
            frames: 0,
            audible: false,
        })
    }

    /// The backend must drain/settle the old cumulative counter before
    /// advancing the fence. Authorization alone never adds frames.
    pub fn authorize(&mut self, ticket: Ticket, audible: bool) -> Result<(), FrameError> {
        if ticket.owner() != self.ticket.owner() || ticket.session() != self.ticket.session() {
            return Err(FrameError::InvalidSession);
        }
        if ticket.revision() <= self.ticket.revision() {
            return Err(FrameError::InvalidRevision);
        }
        self.ticket = ticket;
        self.audible = audible;
        Ok(())
    }

    /// False = stale or duplicate; neither may change totals. A cumulative
    /// counter may only increase under the current audible authorization.
    pub fn observe(&mut self, ticket: Ticket, cumulative_frames: u64) -> Result<bool, FrameError> {
        if ticket != self.ticket {
            return Ok(false);
        }
        if cumulative_frames < self.frames {
            return Err(FrameError::CounterRegressed);
        }
        if cumulative_frames == self.frames {
            return Ok(false);
        }
        if !self.audible {
            return Err(FrameError::RenderedWhileHeld);
        }
        self.frames = cumulative_frames;
        Ok(true)
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Convert once from the entire count, avoiding per-buffer rounding.
    pub fn elapsed_ms(&self) -> u64 {
        ((u128::from(self.frames) * 1_000) / u128::from(self.sample_rate)) as u64
    }
}
