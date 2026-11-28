use std::collections::{BTreeMap, BTreeSet};

use crate::{ListenRecord, Outcome};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SummaryError {
    ConflictingSession,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    /// Sum of track audio time, not wall-clock listening time during crossfade.
    pub track_audio_ms: u128,
    pub sessions: usize,
    pub qualified_plays: usize,
    pub unique_qualified_tracks: usize,
    /// Reaching the natural end is not proof the entire track was heard.
    pub natural_ends: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl Summary {
    /// Identical deliveries are counted once. Conflicting records fail closed.
    /// This is an in-memory reference reducer; persistence comes separately.
    pub fn aggregate<'a>(
        records: impl IntoIterator<Item = &'a ListenRecord>,
    ) -> Result<Self, SummaryError> {
        let mut seen: BTreeMap<&str, &ListenRecord> = BTreeMap::new();
        let mut tracks = BTreeSet::new();
        let mut out = Self::default();
        for record in records {
            if let Some(previous) = seen.get(record.session_id()) {
                if *previous != record {
                    return Err(SummaryError::ConflictingSession);
                }
                continue;
            }
            seen.insert(record.session_id(), record);
            out.sessions += 1;
            out.track_audio_ms += u128::from(record.rendered_ms());
            if record.qualifies() {
                out.qualified_plays += 1;
                tracks.insert(record.track());
            }
            match record.outcome() {
                Outcome::NaturalEnd => out.natural_ends += 1,
                Outcome::Skipped => out.skipped += 1,
                Outcome::Failed => out.failed += 1,
                Outcome::Stopped => {}
            }
        }
        out.unique_qualified_tracks = tracks.len();
        Ok(out)
    }
}
