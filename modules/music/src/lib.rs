//! Standalone music domain. No network, UI, account credentials or disk writes.
#![forbid(unsafe_code)]
mod listening;
pub mod playback;
mod summary;

pub use listening::{ListenRecord, ListenSession, Outcome, SessionError, TrackKey};
pub use summary::{Summary, SummaryError};
