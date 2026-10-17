//! History mutations must go through `HistoryService`: raw store writers
//! are intentionally not part of the cross-crate or sibling-module API.
//! These independent compile-fail doctests detect accidental re-export.
//! ```compile_fail
//! use aquilum_core::history::store;
//! let _mutate = store::adopt_quantum_history;
//! ```
//! ```compile_fail
//! use aquilum_core::history::store;
//! let _mutate = store::write_version;
//! ```
//! ```compile_fail
//! use aquilum_core::history::store;
//! let _mutate = store::update_version_text;
//! ```
//! ```compile_fail
//! use aquilum_core::history::store;
//! let _mutate = store::remove_version_and_names;
//! ```
//! ```compile_fail
//! use aquilum_core::history::store;
//! let _mutate = store::write_name;
//! ```
//! ```compile_fail
//! use aquilum_core::history::store;
//! let _mutate = store::remove_expired;
//! ```

pub mod milestone;
pub mod service;
pub mod store;

pub use milestone::Source;
pub use service::{HistoryService, NoteWrite};
pub use store::{version_at, AQUILUM_FOLDER};
