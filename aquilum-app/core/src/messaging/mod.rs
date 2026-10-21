//! A deterministic, non-networked message-state contract for Akula's built-in
//! communication feature (AK-MSG-CORE-01A).
//!
//! This feature-gated module starts NO worker, opens NO socket or database,
//! and cannot send plaintext or assert that a receipt has a valid signature.
//! The caller MUST persist the same original encrypted WOW bytes before
//! constructing an outgoing state and supply a trusted receipt verifier.
//!
//! This models the sender's *knowledge* of delivery. The actual recipient's
//! durable inbox is implemented later by WOW W4 / AK-MSG-CORE-01C.
//! It is NOT a production crypto, journal, transport or trust implementation.

use crate::sync::DeviceId;

/// Upper bound for opaque authenticated receipt evidence, before verification.
pub const MAX_RECEIPT_PROOF_BYTES: usize = 4096;

/// Identity and digest of the already-durably-queued, immutable WOW envelope.
/// The digest is over its original protected bytes, NOT over transport frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Descriptor {
    pub id: [u8; 16],
    pub recipient: DeviceId,
    pub ciphertext_digest: [u8; 32],
    pub policy_epoch: u64,
    pub expires_at_ms: u64,
}

/// Mutually exclusive phases of an outgoing message's sender-side view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    LocalQueued,
    InTransit,
    RelayStored,
    NeedsRetry,
    RecipientAccepted,
    Expired,
    Revoked,
}

/// Deterministic inputs from the host/transport adapter.
/// `RelayStored` only records a relay observation: it is NEVER final delivery.
/// `TrustRevoked` is authority input, not a peer's unsigned gossip.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    RouteAvailable,
    RouteLost,
    SendAttempt,
    TransportFailed,
    RelayStored,
    TrustRevoked { new_epoch: u64 },
    Tick,
}

/// A candidate signed receipt. Verify the signature AND its trusted mapping
/// to the recipient DeviceId in the injected authoritative verifier.
/// Receipt bytes are not accepted as an authorization token by themselves.
#[derive(Clone, Copy, Debug)]
pub struct ReceiptCandidate<'a> {
    pub id: [u8; 16],
    pub recipient: DeviceId,
    pub ciphertext_digest: [u8; 32],
    pub policy_epoch: u64,
    pub signature: &'a [u8],
}

/// An authoritative adapter must implement cryptographic verification and
/// trusted DeviceId-to-signing-key binding. Test witnesses are NOT security
/// implementations and must never be used for actual message delivery.
pub trait ReceiptVerifier {
    fn verify(&self, candidate: &ReceiptCandidate<'_>) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptOutcome {
    NewlyAccepted,
    DuplicateAlreadyAccepted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidDescriptor,
    TimeReversal,
    Offline,
    InvalidTransition,
    Terminal,
    Expired,
    Revoked,
    StaleEpoch,
    ReceiptMismatch,
    InvalidProof,
}

#[derive(Clone, Debug)]
pub struct OutboundMessage {
    descriptor: Descriptor,
    phase: Phase,
    route_available: bool,
    current_policy_epoch: u64,
    now_ms: u64,
}

impl OutboundMessage {
    /// PRECONDITION: the caller has durably queued the original encrypted
    /// bytes; this pure model has no access to a journal and cannot prove it.
    pub fn after_durable_enqueue(descriptor: Descriptor, queued_at_ms: u64) -> Result<Self, Error> {
        if descriptor.id == [0; 16]
            || descriptor.policy_epoch == 0
            || descriptor.expires_at_ms <= queued_at_ms
        {
            return Err(Error::InvalidDescriptor);
        }
        Ok(Self {
            descriptor,
            phase: Phase::LocalQueued,
            route_available: false,
            current_policy_epoch: descriptor.policy_epoch,
            now_ms: queued_at_ms,
        })
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn route_available(&self) -> bool {
        self.route_available
    }

    pub fn recipient_accepted(&self) -> bool {
        self.phase == Phase::RecipientAccepted
    }

    pub fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }

    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    fn advance(&mut self, now_ms: u64) -> Result<(), Error> {
        if now_ms < self.now_ms {
            return Err(Error::TimeReversal);
        }
        self.now_ms = now_ms;
        if now_ms >= self.descriptor.expires_at_ms
            && !matches!(self.phase, Phase::RecipientAccepted | Phase::Revoked)
        {
            self.phase = Phase::Expired;
        }
        Ok(())
    }

    /// The transition is pure; no I/O, clock reads or background retry occurs.
    /// Rejected inputs cannot promote to a later delivery phase.
    pub fn step(&mut self, now_ms: u64, event: Event) -> Result<Phase, Error> {
        self.advance(now_ms)?;
        match event {
            Event::Tick => return Ok(self.phase),
            Event::TrustRevoked { new_epoch } => {
                if new_epoch <= self.current_policy_epoch {
                    return Err(Error::StaleEpoch);
                }
                self.current_policy_epoch = new_epoch;
                if !matches!(self.phase, Phase::RecipientAccepted | Phase::Expired) {
                    self.phase = Phase::Revoked;
                }
                return Ok(self.phase);
            }
            _ => {}
        }
        match self.phase {
            Phase::Expired => return Err(Error::Expired),
            Phase::Revoked => return Err(Error::Revoked),
            Phase::RecipientAccepted => return Err(Error::Terminal),
            _ => {}
        }

        match event {
            Event::RouteAvailable => self.route_available = true,
            Event::RouteLost => {
                self.route_available = false;
                if self.phase == Phase::InTransit {
                    self.phase = Phase::NeedsRetry;
                }
            }
            Event::SendAttempt => {
                if !self.route_available {
                    return Err(Error::Offline);
                }
                if matches!(
                    self.phase,
                    Phase::LocalQueued | Phase::NeedsRetry | Phase::RelayStored
                ) {
                    self.phase = Phase::InTransit;
                } else {
                    return Err(Error::InvalidTransition);
                }
            }
            Event::TransportFailed => match self.phase {
                Phase::InTransit => self.phase = Phase::NeedsRetry,
                // A relay's earlier storage is not undone by a later link loss.
                Phase::NeedsRetry | Phase::RelayStored => {}
                _ => return Err(Error::InvalidTransition),
            },
            Event::RelayStored => {
                if matches!(
                    self.phase,
                    Phase::InTransit | Phase::NeedsRetry | Phase::RelayStored
                ) {
                    self.phase = Phase::RelayStored;
                } else {
                    return Err(Error::InvalidTransition);
                }
            }
            Event::Tick | Event::TrustRevoked { .. } => unreachable!(),
        }
        Ok(self.phase)
    }

    /// Only the recipient's authenticated, policy-current, durable-acceptance
    /// receipt may advance this state. A relay receipt and transport ACK cannot.
    ///
    /// The caller-supplied verifier MUST validate an actual W4 signature using
    /// the trusted signing key bound to Descriptor::recipient. This module
    /// deliberately offers no permissive default verifier.
    pub fn accept_receipt(
        &mut self,
        now_ms: u64,
        receipt: &ReceiptCandidate<'_>,
        verifier: &impl ReceiptVerifier,
    ) -> Result<ReceiptOutcome, Error> {
        self.advance(now_ms)?;
        if receipt.policy_epoch != self.current_policy_epoch
            || self.descriptor.policy_epoch != self.current_policy_epoch
        {
            return Err(Error::StaleEpoch);
        }
        match self.phase {
            Phase::Expired => return Err(Error::Expired),
            Phase::Revoked => return Err(Error::Revoked),
            _ => {}
        }
        if receipt.id != self.descriptor.id
            || receipt.recipient != self.descriptor.recipient
            || receipt.ciphertext_digest != self.descriptor.ciphertext_digest
        {
            return Err(Error::ReceiptMismatch);
        }
        if receipt.signature.is_empty() || receipt.signature.len() > MAX_RECEIPT_PROOF_BYTES {
            return Err(Error::InvalidProof);
        }
        if !verifier.verify(receipt) {
            return Err(Error::InvalidProof);
        }
        if self.phase == Phase::RecipientAccepted {
            return Ok(ReceiptOutcome::DuplicateAlreadyAccepted);
        }
        if self.phase == Phase::LocalQueued {
            // An untransmitted message cannot receive a genuine final ACK.
            return Err(Error::InvalidTransition);
        }
        self.phase = Phase::RecipientAccepted;
        Ok(ReceiptOutcome::NewlyAccepted)
    }
}

#[cfg(test)]
mod scale_tests;
#[cfg(test)]
mod tests;
