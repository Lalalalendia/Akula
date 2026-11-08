//! Non-authorizing display label obtained ONLY from a verified signed V2 receipt.
//! The sender's original disk filename is not attested; the label is still its
//! display-only claim, never a trusted pathname, extension or file authority.
use super::{verify_cold_receipt_v2, ProofError};
use crate::p2p_offer_lab::receipt_binding::ObservedLocalReceipt;
use crate::p2p_offer_lab::{OfferV1, PeerBinding};
use uuid::Uuid;

/// A label linked to one verified offer/receipt, not a transferable permission.
/// No filesystem, network, file-open or Vault operations are exposed here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedOfferLabelV2 {
    receipt_id: Uuid,
    text: String,
}

impl VerifiedOfferLabelV2 {
    pub fn receipt_id(&self) -> Uuid {
        self.receipt_id
    }

    pub fn as_display_text(&self) -> &str {
        &self.text
    }
}

/// Verify both signatures, the complete Offer commitment and the *observed*
/// locally rehashed file facts before returning the selected display label.
/// Caller must independently pin both peers and the local Inbox HANDLE.
/// Do not use with legacy content-only Inbox IDs or raw network JSON.
/// No durable consent or OS-owned proof journal is asserted by this helper.
pub fn verified_offer_label_v2(
    signed_record: &[u8],
    stored_offer: &OfferV1,
    pinned_sender: PeerBinding,
    pinned_receiver: PeerBinding,
    rehashed_local: ObservedLocalReceipt,
    now: u64,
) -> Result<VerifiedOfferLabelV2, ProofError> {
    let checked = verify_cold_receipt_v2(
        signed_record,
        stored_offer,
        pinned_sender,
        pinned_receiver,
        rehashed_local,
        now,
    )?;
    // This lookup is safe only AFTER signature and commitment verification:
    // the label itself is included in the signed semantic commitment.
    let selected = stored_offer
        .files
        .get(usize::from(checked.selected_index))
        .ok_or(ProofError::WrongSelection)?;
    Ok(VerifiedOfferLabelV2 {
        receipt_id: checked.receipt_id,
        text: selected.label.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture;
    use super::*;

    const NOW: u64 = 1_800_000_000;

    #[test]
    fn cold_v2_proof_binds_display_label_to_selected_receipt() {
        let (offer, record, observed) = fixture();
        let label = verified_offer_label_v2(
            &record,
            &offer,
            offer.sender,
            offer.receiver,
            observed,
            NOW + 200,
        )
        .unwrap();
        assert_eq!(label.receipt_id(), observed.receipt_id);
        assert_eq!(label.as_display_text(), "signed-report.txt");
    }

    #[test]
    fn altered_label_or_untrusted_path_cannot_reuse_signed_commitment() {
        let (offer, record, observed) = fixture();
        for text in ["renamed-report.txt", "../private.txt", "CON", "looks.pdf"] {
            let mut changed = offer.clone();
            changed.files[0].label = text.into();
            assert_eq!(
                verified_offer_label_v2(
                    &record,
                    &changed,
                    offer.sender,
                    offer.receiver,
                    observed,
                    NOW + 6,
                ),
                Err(ProofError::WrongContext),
            );
        }
    }

    #[test]
    fn forged_signatures_and_wrong_local_receipt_are_not_display_labels() {
        let (offer, record, observed) = fixture();
        let mut forged = record;
        let last_byte = forged.len() - 1;
        forged[last_byte] ^= 1;
        assert_eq!(
            verified_offer_label_v2(
                &forged,
                &offer,
                offer.sender,
                offer.receiver,
                observed,
                NOW + 6,
            ),
            Err(ProofError::BadSignature),
        );
        let mut wrong = observed;
        wrong.receipt_id = Uuid::new_v4();
        assert_eq!(
            verified_offer_label_v2(
                &record,
                &offer,
                offer.sender,
                offer.receiver,
                wrong,
                NOW + 6,
            ),
            Err(ProofError::WrongReceipt),
        );
        wrong = observed;
        wrong.blake3_root[0] ^= 1;
        assert_eq!(
            verified_offer_label_v2(
                &record,
                &offer,
                offer.sender,
                offer.receiver,
                wrong,
                NOW + 6,
            ),
            Err(ProofError::WrongReceipt),
        );
    }

    #[test]
    fn wrong_peer_or_time_never_yields_a_display_label() {
        let (offer, record, observed) = fixture();
        let mut different = offer.receiver;
        different.endpoint_id[0] ^= 1;
        assert_eq!(
            verified_offer_label_v2(&record, &offer, offer.sender, different, observed, NOW + 6),
            Err(ProofError::WrongContext),
        );
        assert_eq!(
            verified_offer_label_v2(
                &record,
                &offer,
                offer.sender,
                offer.receiver,
                observed,
                NOW + 2,
            ),
            Err(ProofError::InvalidTime),
        );
        assert_eq!(
            verified_offer_label_v2(&[], &offer, offer.sender, offer.receiver, observed, NOW + 6),
            Err(ProofError::BadLength),
        );
    }
}
