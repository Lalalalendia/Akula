//! Narrow authenticated Iroh QUIC NodeV1 Put/Get transport (non-shipping).
//! The service owner must supply a real completed Connection, a provisioned
//! NodeHost and independently pinned client endpoint/owner grants. No 0-RTT,
//! no plaintext fallback, no request-selected server policy or File Gate write.
mod client;
mod server;
mod wire;

pub use client::{get_over_authenticated_iroh, put_over_authenticated_iroh};
pub use server::serve_one_authenticated_connection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeWireDenied;
