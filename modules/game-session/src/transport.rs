//! A bounded frame channel independent of Roha/noq, WebSocket or Bluetooth.
//!
//! A production adapter MUST report the authenticated connection's peer identity
//! as ReceivedFrame::from. Never take it from the untrusted frame payload.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use crate::PeerId;

pub const MAX_FRAME_BYTES: usize = 256;
pub const MAX_QUEUED_FRAMES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivedFrame {
    /// This identity is supplied by the transport, not by packet data.
    pub from: PeerId,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    InvalidPeers,
    WrongDestination,
    EmptyFrame,
    OversizedFrame,
    QueueFull,
    Closed,
    Poisoned,
}

/// Inbound/outbound framed channel API.
/// Async QUIC/WebSocket handlers can bridge these operations from their own I/O tasks.
pub trait GameTransport {
    fn local_peer(&self) -> PeerId;
    fn send(&mut self, destination: PeerId, bytes: &[u8]) -> Result<(), TransportError>;
    fn try_recv(&mut self) -> Result<Option<ReceivedFrame>, TransportError>;
}

#[derive(Debug, Default)]
struct Inbox {
    messages: VecDeque<ReceivedFrame>,
    closed: bool,
}

/// One endpoint of a two-device in-process laboratory link.
/// This is NOT a network socket, peer authentication or production room.
#[derive(Debug)]
pub struct InMemoryTransport {
    local: PeerId,
    remote: PeerId,
    incoming: Arc<Mutex<Inbox>>,
    outgoing: Arc<Mutex<Inbox>>,
    closed: bool,
}

impl InMemoryTransport {
    pub fn pair(a: PeerId, b: PeerId) -> Result<(Self, Self), TransportError> {
        if a == b || a.0 == [0; 32] || b.0 == [0; 32] {
            return Err(TransportError::InvalidPeers);
        }
        let a_inbox = Arc::new(Mutex::new(Inbox::default()));
        let b_inbox = Arc::new(Mutex::new(Inbox::default()));
        Ok((
            Self {
                local: a,
                remote: b,
                incoming: Arc::clone(&a_inbox),
                outgoing: Arc::clone(&b_inbox),
                closed: false,
            },
            Self {
                local: b,
                remote: a,
                incoming: b_inbox,
                outgoing: a_inbox,
                closed: false,
            },
        ))
    }

    pub fn queued_frames(&self) -> Result<usize, TransportError> {
        let inbox = self.incoming.lock().map_err(|_| TransportError::Poisoned)?;
        Ok(inbox.messages.len())
    }

    /// Closing an endpoint drops its unread frames and refuses new deliveries to it.
    pub fn close(&mut self) -> Result<(), TransportError> {
        if self.closed {
            return Ok(());
        }
        let mut inbox = self.incoming.lock().map_err(|_| TransportError::Poisoned)?;
        inbox.messages.clear();
        inbox.closed = true;
        self.closed = true;
        Ok(())
    }
}

impl GameTransport for InMemoryTransport {
    fn local_peer(&self) -> PeerId {
        self.local
    }

    fn send(&mut self, destination: PeerId, bytes: &[u8]) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        if destination != self.remote {
            return Err(TransportError::WrongDestination);
        }
        if bytes.is_empty() {
            return Err(TransportError::EmptyFrame);
        }
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(TransportError::OversizedFrame);
        }
        let mut inbox = self.outgoing.lock().map_err(|_| TransportError::Poisoned)?;
        if inbox.closed {
            return Err(TransportError::Closed);
        }
        if inbox.messages.len() == MAX_QUEUED_FRAMES {
            return Err(TransportError::QueueFull);
        }
        inbox.messages.push_back(ReceivedFrame {
            from: self.local,
            payload: bytes.to_vec(),
        });
        Ok(())
    }

    fn try_recv(&mut self) -> Result<Option<ReceivedFrame>, TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        let mut inbox = self.incoming.lock().map_err(|_| TransportError::Poisoned)?;
        Ok(inbox.messages.pop_front())
    }
}
