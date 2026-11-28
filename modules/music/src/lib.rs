//! Standalone music domain and local statistics; no network, UI or credentials.
#![forbid(unsafe_code)]
mod listening;
pub mod storage;
mod summary;

pub use listening::{ListenRecord, ListenSession, Outcome, SessionError, TrackKey};
pub use summary::{Summary, SummaryError};
