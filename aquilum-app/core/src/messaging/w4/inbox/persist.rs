//! Reusable atomic W4 inbox+receipt writer. Only Blue-authenticated and
//! explicitly trusted callers may supply AuthenticatedW3. The caller owns
//! the transaction and holds the authorization fence until COMMIT.

use ring::signature::Ed25519KeyPair;
use rusqlite::{params, OptionalExtension};

use super::{
    AuthenticatedW3, InboundResult, InboundStatus, W4Inbox, MAX_INBOX_PER_SENDER, MAX_INBOX_ROWS,
};
use crate::messaging::w4::{Error, DOMAIN, RECEIPT_LEN};

impl W4Inbox {
    /// Common SQLite receipt writer. Callers must authenticate the input,
    /// check the current policy, and (for real network traffic) hold the
    /// same-DB IMMEDIATE authorization/revocation fence transaction.
    pub(super) fn persist_in_transaction(
        tx: rusqlite::Transaction<'_>,
        input: &AuthenticatedW3<'_>,
        signer: &Ed25519KeyPair,
    ) -> Result<InboundResult, Error> {
        let sender = input.sender.as_uuid().into_bytes();
        let recipient = input.recipient.as_uuid().into_bytes();
        let existing: Option<(Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, i64, i64)> = tx
            .query_row(
                "SELECT digest,ciphertext,receipt,recipient,epoch,expiry_ms
                 FROM messaging_w4_inbox WHERE sender=?1 AND message=?2",
                params![sender.as_slice(), input.message.as_slice()],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .optional()?;
        if let Some((digest, body, receipt, saved_recipient, epoch, expiry)) = existing {
            if digest.as_slice() != input.digest.as_slice()
                || body.as_slice() != input.packet
                || saved_recipient.as_slice() != recipient.as_slice()
                || epoch != input.epoch as i64
                || expiry != input.expires_at_ms as i64
            {
                return Err(Error::Conflict);
            }
            tx.commit()?;
            return Ok(InboundResult {
                status: InboundStatus::Duplicate,
                signed_receipt: receipt,
            });
        }

        // Bound total disk use and one sender's share before making a new
        // record; an existing duplicate remains readable when fully occupied.
        let total: i64 =
            tx.query_row("SELECT COUNT(*) FROM messaging_w4_inbox", [], |r| r.get(0))?;
        let sender_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM messaging_w4_inbox WHERE sender=?1",
            params![sender.as_slice()],
            |r| r.get(0),
        )?;
        if total >= MAX_INBOX_ROWS || sender_count >= MAX_INBOX_PER_SENDER {
            return Err(Error::MailboxFull);
        }

        // WOW W4's exact domain and 73-byte signed header. The proof only
        // becomes externally observable after the ciphertext+proof COMMIT.
        let mut receipt = Vec::with_capacity(RECEIPT_LEN);
        receipt.push(1);
        receipt.extend_from_slice(&recipient);
        receipt.extend_from_slice(&input.message);
        receipt.extend_from_slice(&input.digest);
        receipt.extend_from_slice(&input.epoch.to_be_bytes());
        let mut tbs = Vec::with_capacity(DOMAIN.len() + receipt.len());
        tbs.extend_from_slice(DOMAIN);
        tbs.extend_from_slice(&receipt);
        receipt.extend_from_slice(signer.sign(&tbs).as_ref());
        if receipt.len() != RECEIPT_LEN {
            return Err(Error::InvalidReceipt);
        }

        tx.execute(
            "INSERT INTO messaging_w4_inbox(
                sender,recipient,message,digest,ciphertext,epoch,expiry_ms,receipt
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                sender.as_slice(),
                recipient.as_slice(),
                input.message.as_slice(),
                input.digest.as_slice(),
                input.packet,
                input.epoch as i64,
                input.expires_at_ms as i64,
                &receipt,
            ],
        )?;
        tx.commit()?;
        Ok(InboundResult {
            status: InboundStatus::Stored,
            signed_receipt: receipt,
        })
    }
}
