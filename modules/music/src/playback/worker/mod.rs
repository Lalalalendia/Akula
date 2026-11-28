//! Deterministic reference worker. No threads, device, filesystem or network.

mod backend;
mod mailbox;
mod runtime;

pub use backend::{BackendError, MockBackend};
pub use mailbox::{Command, Mailbox, SubmitError, COMMAND_CAPACITY};
pub use runtime::{Runtime, RuntimeError};
