use super::{Effect, Error, Event, Phase, Player, Ticket};

impl Player {
    /// Late, duplicate or wrong-phase events are ignored. Current events
    /// may emit a fresh command; only that ticket may drive later progress.
    /// Playback phase here means requested/acknowledged reducer state, not
    /// measured rendered audio. Statistics must come from the output backend.
    pub fn receive(&mut self, ticket: Ticket, event: Event) -> Result<Option<Effect>, Error> {
        if ticket != self.ticket || matches!(self.phase, Phase::Idle | Phase::Failed) {
            return Ok(None);
        }
        match (self.phase, event) {
            (Phase::Loading, Event::Ready) => {
                let ticket = self.next_ticket(false)?;
                Ok(Some(self.install_pause(self.want_paused, ticket)))
            }
            (Phase::Seeking, Event::Seeked { position_ms }) => {
                self.validate_position(position_ms)?;
                let ticket = self.next_ticket(false)?;
                self.position_ms = position_ms;
                Ok(Some(self.install_pause(self.want_paused, ticket)))
            }
            (Phase::Playing, Event::Progress { position_ms }) => {
                self.validate_position(position_ms)?;
                // Reordered progress in the same revision cannot move back.
                self.position_ms = self.position_ms.max(position_ms);
                Ok(None)
            }
            (Phase::Playing, Event::Ended) => Ok(Some(self.advance(true)?)),
            (_, Event::Failed) => {
                let ticket = self.next_ticket(false)?;
                self.ticket = ticket;
                self.phase = Phase::Failed;
                Ok(Some(Effect::Stop { ticket }))
            }
            _ => Ok(None),
        }
    }

    fn validate_position(&self, position_ms: u64) -> Result<(), Error> {
        if position_ms > self.current().ok_or(Error::NoTrack)?.duration_ms() {
            return Err(Error::InvalidPosition);
        }
        Ok(())
    }
}
