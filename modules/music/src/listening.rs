//! Count rendered audio, not elapsed wall time or the seek position.

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TrackKey {
    provider: String,
    id: String,
}

impl TrackKey {
    pub fn new(provider: &str, id: &str) -> Result<Self, SessionError> {
        if provider.trim().is_empty() || id.trim().is_empty() {
            return Err(SessionError::EmptyIdentity);
        }
        Ok(Self {
            provider: provider.to_owned(),
            id: id.to_owned(),
        })
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    NaturalEnd,
    Skipped,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionError {
    EmptyIdentity,
    InvalidDuration,
    CounterRegressed,
    RenderedWhilePaused,
}

/// An instance of playback, including repeats. The host supplies a unique ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListenRecord {
    session_id: String,
    track: TrackKey,
    duration_ms: u64,
    rendered_ms: u64,
    seeks: u32,
    pauses: u32,
    outcome: Outcome,
}

impl ListenRecord {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn track(&self) -> &TrackKey {
        &self.track
    }

    pub fn rendered_ms(&self) -> u64 {
        self.rendered_ms
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    pub fn seeks(&self) -> u32 {
        self.seeks
    }

    pub fn pauses(&self) -> u32 {
        self.pauses
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// Initial local policy: at least 30 seconds, or the duration of a short track.
    /// This is not a provider scrobble rule or proof of uninterrupted listening.
    pub fn qualifies(&self) -> bool {
        self.rendered_ms >= self.duration_ms.min(30_000)
    }
}

/// Single-owner reducer. Progress is a cumulative counter of rendered audio
/// for this session, independent of media position, pauses and buffering.
/// The future output backend must supply that counter and fence stale sessions.
pub struct ListenSession {
    session_id: String,
    track: TrackKey,
    duration_ms: u64,
    rendered_ms: u64,
    seeks: u32,
    pauses: u32,
    paused: bool,
}

impl ListenSession {
    pub fn new(session_id: &str, track: TrackKey, duration_ms: u64) -> Result<Self, SessionError> {
        if session_id.trim().is_empty() {
            return Err(SessionError::EmptyIdentity);
        }
        if duration_ms == 0 {
            return Err(SessionError::InvalidDuration);
        }
        Ok(Self {
            session_id: session_id.to_owned(),
            track,
            duration_ms,
            rendered_ms: 0,
            seeks: 0,
            pauses: 0,
            paused: false,
        })
    }

    /// Duplicate cumulative observations are harmless. Invalid observations
    /// leave state untouched. No time is inferred from a seek or a pause.
    pub fn observe_rendered(&mut self, total_ms: u64) -> Result<bool, SessionError> {
        if total_ms < self.rendered_ms {
            return Err(SessionError::CounterRegressed);
        }
        if total_ms == self.rendered_ms {
            return Ok(false);
        }
        if self.paused {
            return Err(SessionError::RenderedWhilePaused);
        }
        self.rendered_ms = total_ms;
        Ok(true)
    }

    pub fn set_paused(&mut self, paused: bool) {
        if paused && !self.paused {
            self.pauses = self.pauses.saturating_add(1);
        }
        self.paused = paused;
    }

    /// Call once per applied seek. Seeking never changes rendered time.
    pub fn note_seek(&mut self) {
        self.seeks = self.seeks.saturating_add(1);
    }

    /// A replay gets a new session. Finishing consumes this session.
    pub fn finish(self, outcome: Outcome) -> ListenRecord {
        ListenRecord {
            session_id: self.session_id,
            track: self.track,
            duration_ms: self.duration_ms,
            rendered_ms: self.rendered_ms,
            seeks: self.seeks,
            pauses: self.pauses,
            outcome,
        }
    }
}
