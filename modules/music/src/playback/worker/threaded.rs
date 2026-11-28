//! Threaded reference transport; only short mock work runs under the lock.
//! A real audio callback/network task must never use this lock or wait here.

use std::collections::VecDeque;
use std::io;
use std::num::NonZeroU64;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::playback::{Event, MAX_QUEUE, Phase, Ticket};

use super::{COMMAND_CAPACITY, Command, Runtime, RuntimeError, SubmitError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    Intent(Command),
    Complete { ticket: Ticket, success: bool },
    Notify { ticket: Ticket, event: Event },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Admission {
    Closed,
    WrongOwner,
    InvalidQueue,
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkError {
    Submit(SubmitError),
    Runtime(RuntimeError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Closed;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub ticket: Ticket,
    pub phase: Phase,
    pub position_ms: u64,
    pub tracks: usize,
    pub queued: usize,
    pub playing: bool,
    pub pending: bool,
    pub last_error: Option<WorkError>,
}

struct State {
    runtime: Runtime,
    queue: VecDeque<Request>,
    closing: bool,
    last_error: Option<WorkError>,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

/// One owner thread. For account switch, shut down and join this worker
/// before creating a new worker with a globally fresh owner ID.
pub struct ThreadedRuntime {
    owner: NonZeroU64,
    shared: Arc<Shared>,
    join: Option<JoinHandle<()>>,
}

impl ThreadedRuntime {
    pub fn spawn(owner: NonZeroU64) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                runtime: Runtime::new(owner),
                queue: VecDeque::with_capacity(COMMAND_CAPACITY),
                closing: false,
                last_error: None,
            }),
            ready: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        let join = thread::Builder::new()
            .name("akula-music-mock".into())
            .spawn(move || drive(owner, &worker))?;
        Ok(Self {
            owner,
            shared,
            join: Some(join),
        })
    }

    /// Bounded admission, not a blocking wait for space. The lock protects
    /// only the small reference reducer; a real backend needs separate I/O.
    /// Rejected requests remain owned by the caller.
    pub fn submit(&self, owner: NonZeroU64, request: Request) -> Result<(), (Admission, Request)> {
        if owner != self.owner || match &request {
            Request::Complete { ticket, .. } | Request::Notify { ticket, .. } => ticket.owner() != owner,
            Request::Intent(_) => false,
        } {
            return Err((Admission::WrongOwner, request));
        }
        if let Request::Intent(Command::Replace { tracks, start }) = &request {
            if tracks.len() > MAX_QUEUE || *start >= tracks.len() {
                return Err((Admission::InvalidQueue, request));
            }
        }
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(_) => return Err((Admission::Closed, request)),
        };
        if state.closing {
            return Err((Admission::Closed, request));
        }
        if matches!(&request, Request::Intent(Command::Stop)) {
            state.queue.clear();
        } else if state.queue.len() >= COMMAND_CAPACITY {
            return Err((Admission::Full, request));
        }
        state.queue.push_back(request);
        self.shared.ready.notify_all();
        Ok(())
    }

    pub fn snapshot(&self) -> Result<Snapshot, Closed> {
        let state = self.shared.state.lock().map_err(|_| Closed)?;
        Ok(Snapshot {
            ticket: state.runtime.player().ticket(),
            phase: state.runtime.player().phase(),
            position_ms: state.runtime.player().position_ms(),
            tracks: state.runtime.player().queue_len(),
            queued: state.queue.len(),
            playing: state.runtime.backend().playing(),
            pending: state.runtime.backend().pending(),
            last_error: state.last_error,
        })
    }

    /// For tests and controlled callers, not the UI/audio callback. Draining
    /// the request queue does not mean an outstanding load has completed.
    pub fn wait_idle(&self, timeout: Duration) -> Result<bool, Closed> {
        let state = self.shared.state.lock().map_err(|_| Closed)?;
        let (state, _) = self.shared.ready.wait_timeout_while(state, timeout, |s| {
            !s.closing && !s.queue.is_empty()
        }).map_err(|_| Closed)?;
        Ok(state.queue.is_empty())
    }

    pub fn shutdown(mut self) -> thread::Result<()> {
        self.close();
        self.join.take().map_or(Ok(()), JoinHandle::join)
    }

    fn close(&self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closing = true;
        state.queue.clear();
        self.shared.ready.notify_all();
    }
}

impl Drop for ThreadedRuntime {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            self.close();
            let _ = join.join();
        }
    }
}

fn drive(owner: NonZeroU64, shared: &Shared) {
    loop {
        let Ok(state) = shared.state.lock() else { return };
        let Ok(mut state) = shared.ready.wait_while(state, |s| {
            !s.closing && s.queue.is_empty()
        }) else { return };
        if state.closing {
            if state.runtime.submit(owner, Command::Stop).is_ok() {
                let _ = state.runtime.step();
            }
            shared.ready.notify_all();
            return;
        }
        if let Some(request) = state.queue.pop_front() {
            let result = execute(owner, &mut state.runtime, request);
            if let Err(error) = result {
                state.last_error = Some(error);
            }
        }
        shared.ready.notify_all();
        // State is dropped here; producers may submit before the next step.
    }
}

fn execute(owner: NonZeroU64, runtime: &mut Runtime, request: Request) -> Result<(), WorkError> {
    match request {
        Request::Intent(command) => {
            runtime.submit(owner, command).map_err(|(reason, _)| WorkError::Submit(reason))?;
            runtime.step().map_err(WorkError::Runtime)?;
        }
        Request::Complete { ticket, success } => {
            runtime.complete(ticket, success).map_err(WorkError::Runtime)?;
        }
        Request::Notify { ticket, event } => {
            runtime.notify(ticket, event).map_err(WorkError::Runtime)?;
        }
    }
    Ok(())
}
