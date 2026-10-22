//! Test-only Akula consent -> existing Staya preview AUTHORIZE handoff.
//!
//! This is deliberately NOT a production grant or an authenticated local IPC.
//! It runs only in the separate non-shipping interop crate behind an opt-in
//! feature. The caller supplies test-only "authenticated" PeerBindings;
//! production must independently prove them with live Iroh/OS pins and
//! Core-owned Vault, sender/file and device authorization.
//!
//! The Staya preview currently reads AUTHORIZE from its parent-owned stdin
//! and starts with ephemeral QUIC keys. This module MUST NOT be wired into
//! the installed Akula sender until those separate trust boundaries exist.

use std::collections::BTreeSet;

use aquilum_core::p2p_offer_lab::{DecisionV1, OfferV1, PeerBinding};

use crate::{BridgeError, PreparedBatch, prepare_staya_batch, verify_staya_invitation};

const MAX_LOCAL_ISSUANCES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabArmError {
    Offer(BridgeError),
    ChangedQueue,
    AlreadyIssued,
    BudgetExceeded,
    WrongReceiver,
    Expired,
    AlreadyConsumed,
}

impl From<BridgeError> for LabArmError {
    fn from(error: BridgeError) -> Self {
        Self::Offer(error)
    }
}

/// Per-process test fence. It is NOT a durable replay or revocation witness.
/// No method can authorize a Vault/History/File Gate operation.
#[derive(Default)]
pub struct LabSenderFence {
    issued: BTreeSet<([u8; 16], [u8; 16])>,
}

/// Single-use, bounded, explicitly synthetic operator command.
/// A public Staya invitation is never sufficient to construct this.
pub struct LabSenderArm {
    receiver: [u8; 32],
    queue_commitment: [u8; 32],
    deadline_unix: u64,
    consumed: bool,
}

impl LabSenderFence {
    /// Recheck the SAME accepted Akula Offer/Decision, selected Staya queue
    /// and actual Staya invitation before passing any command to its gated
    /// provider process. These in-memory checks do not authenticate Core.
    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        &mut self,
        offer: &OfferV1,
        decision: &DecisionV1,
        selected: &PreparedBatch,
        invitation: &str,
        trusted_sender: PeerBinding,
        trusted_receiver: PeerBinding,
        now_unix: u64,
    ) -> Result<LabSenderArm, LabArmError> {
        if now_unix >= offer.expires_unix {
            return Err(LabArmError::Expired);
        }
        let issuance_key = (*offer.offer_id.as_bytes(), offer.nonce);
        if self.issued.contains(&issuance_key) {
            return Err(LabArmError::AlreadyIssued);
        }
        if self.issued.len() >= MAX_LOCAL_ISSUANCES {
            return Err(LabArmError::BudgetExceeded);
        }

        let checked = prepare_staya_batch(
            offer.clone(),
            decision,
            trusted_sender,
            trusted_receiver,
            now_unix,
        )?;
        if checked.offer_indexes != selected.offer_indexes
            || checked.offer_commitment != selected.offer_commitment
            || checked.queue.commitment() != selected.queue.commitment()
            || checked.queue.entries() != selected.queue.entries()
        {
            return Err(LabArmError::ChangedQueue);
        }
        verify_staya_invitation(&checked, invitation, trusted_sender.endpoint_id)?;

        self.issued.insert(issuance_key);
        Ok(LabSenderArm {
            receiver: trusted_receiver.endpoint_id,
            queue_commitment: checked.queue.commitment(),
            deadline_unix: offer.expires_unix,
            consumed: false,
        })
    }
}

impl LabSenderArm {
    /// The caller must supply an independently observed receiver key; the
    /// raw CONNECT_BATCH ticket is never accepted as a recipient witness.
    ///
    /// This line is valid ONLY for the existing private parent->child stdin
    /// test controller. It is NOT a signed capability or a host-owned grant.
    pub fn take_operator_line(
        &mut self,
        independently_observed_receiver: [u8; 32],
        now_unix: u64,
    ) -> Result<String, LabArmError> {
        if self.consumed {
            return Err(LabArmError::AlreadyConsumed);
        }
        if now_unix >= self.deadline_unix {
            return Err(LabArmError::Expired);
        }
        if independently_observed_receiver != self.receiver {
            return Err(LabArmError::WrongReceiver);
        }
        self.consumed = true;
        Ok(format!(
            "AUTHORIZE {} {}",
            hex(&self.queue_commitment),
            hex(&self.receiver)
        ))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_receiver_cannot_consume_a_lab_arm() {
        let mut arm = LabSenderArm {
            receiver: [0x17; 32],
            queue_commitment: [0x81; 32],
            deadline_unix: 100,
            consumed: false,
        };
        assert_eq!(
            arm.take_operator_line([0x18; 32], 99),
            Err(LabArmError::WrongReceiver)
        );
        let command = arm.take_operator_line([0x17; 32], 99).unwrap();
        assert_eq!(
            command,
            format!("AUTHORIZE {} {}", "81".repeat(32), "17".repeat(32))
        );
        assert_eq!(
            arm.take_operator_line([0x17; 32], 99),
            Err(LabArmError::AlreadyConsumed)
        );
    }

    #[test]
    fn an_expired_lab_arm_never_emits_an_authorize_line() {
        let mut arm = LabSenderArm {
            receiver: [0x17; 32],
            queue_commitment: [0x81; 32],
            deadline_unix: 100,
            consumed: false,
        };
        assert_eq!(
            arm.take_operator_line([0x17; 32], 100),
            Err(LabArmError::Expired)
        );
        assert_eq!(
            arm.take_operator_line([0x17; 32], 101),
            Err(LabArmError::Expired)
        );
    }
}
