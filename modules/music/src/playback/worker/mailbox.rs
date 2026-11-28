use std::collections::VecDeque;
use std::num::NonZeroU64;

use crate::playback::{MAX_QUEUE, Repeat, Track};

pub const COMMAND_CAPACITY: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Replace { tracks: Vec<Track>, start: usize },
    Next,
    Previous,
    Pause(bool),
    Seek(u64),
    Retry,
    Repeat(Repeat),
    Stop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitError {
    WrongOwner,
    InvalidQueue,
    Full,
}

/// The UI queues intent, not effects already committed to the reducer.
/// Stop explicitly cancels pending intents and is always admitted.
pub struct Mailbox {
    owner: NonZeroU64,
    queue: VecDeque<Command>,
}

impl Mailbox {
    pub fn new(owner: NonZeroU64) -> Self {
        Self {
            owner,
            queue: VecDeque::with_capacity(COMMAND_CAPACITY),
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// The caller retains the command on rejection and may retry later.
    pub fn submit(&mut self, owner: NonZeroU64, command: Command) -> Result<(), (SubmitError, Command)> {
        if owner != self.owner {
            return Err((SubmitError::WrongOwner, command));
        }
        if let Command::Replace { tracks, start } = &command {
            if tracks.len() > MAX_QUEUE || *start >= tracks.len() {
                return Err((SubmitError::InvalidQueue, command));
            }
        }
        if command == Command::Stop {
            self.queue.clear();
        } else if self.queue.len() == COMMAND_CAPACITY {
            return Err((SubmitError::Full, command));
        }
        self.queue.push_back(command);
        Ok(())
    }

    pub(super) fn pop(&mut self) -> Option<Command> {
        self.queue.pop_front()
    }
}
