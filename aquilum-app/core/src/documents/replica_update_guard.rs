//! Allocation preflight for the *outer header* of a Yrs v1 Update.
//!
//! Yrs 0.28.0 reserves memory from declared client and block counts before it
//! establishes that the wire payload contains enough bytes (upstream #675).
//! This checks only provable length invariants; it is NOT a full Yrs validator.
//! Raw updates from untrusted peers still require an upstream decoder fix.
use super::{failure, update_decoder, ReplicaError};
use yrs::Update;

pub(super) fn decode(update: &[u8]) -> Result<Update, ReplicaError> {
    check(update)?;
    update_decoder::decode(update).map_err(failure)
}

// Mirror Yrs 0.28.0's permissive varint semantics (including wrapped shifts).
// Its decoder accepts overlong varints for Yjs interoperability; the guard
// must not reject valid updates solely because the encoding is noncanonical.
fn varuint(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut number = 0u64;
    for index in 0..11 {
        let octet = *bytes.get(*cursor)?;
        *cursor += 1;
        number |= u64::from(octet & 0x7f).wrapping_shl((index * 7) as u32);
        if octet & 0x80 == 0 {
            return Some(number);
        }
    }
    None
}

fn varuint32(bytes: &[u8], cursor: &mut usize) -> Option<u32> {
    let mut number = 0u32;
    for index in 0..11 {
        let octet = *bytes.get(*cursor)?;
        *cursor += 1;
        number |= u32::from(octet & 0x7f).wrapping_shl((index * 7) as u32);
        if octet & 0x80 == 0 {
            return Some(number);
        }
    }
    None
}

fn fits(count: u32, available: usize, minimum_item_bytes: usize) -> bool {
    (count as u64) <= (available / minimum_item_bytes) as u64
}

pub(super) fn check(update: &[u8]) -> Result<(), ReplicaError> {
    let mut cursor = 0;
    let client_count = varuint32(update, &mut cursor)
        .ok_or_else(|| failure("invalid Yrs v1 client count"))?;
    // Each client requires at least block-count, client-id and clock varints.
    // Leave one byte for the mandatory delete-set header.
    if !fits(client_count, update.len().saturating_sub(cursor + 1), 3) {
        return Err(failure("Yrs v1 client count exceeds encoded input"));
    }
    if client_count > 0 {
        let blocks = varuint32(update, &mut cursor)
            .ok_or_else(|| failure("invalid first Yrs v1 block count"))?;
        varuint(update, &mut cursor)
            .ok_or_else(|| failure("invalid first Yrs v1 client ID"))?;
        varuint32(update, &mut cursor)
            .ok_or_else(|| failure("invalid first Yrs v1 clock"))?;
        // Each block has at least its one-byte info tag.
        if !fits(blocks, update.len().saturating_sub(cursor + 1), 1) {
            return Err(failure("first Yrs v1 block count exceeds encoded input"));
        }
    } else {
        // For an update containing only deletions, the delete set begins here.
        let delete_clients = varuint32(update, &mut cursor)
            .ok_or_else(|| failure("invalid Yrs v1 delete-set count"))?;
        if !fits(delete_clients, update.len() - cursor, 2) {
            return Err(failure("Yrs v1 delete-set count exceeds encoded input"));
        }
        if delete_clients > 0 {
            varuint(update, &mut cursor)
                .ok_or_else(|| failure("invalid first delete-set client ID"))?;
            let ranges = varuint32(update, &mut cursor)
                .ok_or_else(|| failure("invalid first delete-set range count"))?;
            if !fits(ranges, update.len() - cursor, 2) {
                return Err(failure("first delete-set range count exceeds encoded input"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::merge::TextEdit;
    use crate::documents::replica::Replica;

    #[test]
    fn rejects_count_claims_without_entering_the_yrs_allocator() {
        let huge = [0xff, 0xff, 0xff, 0xff, 0x0f];
        let mut clients = huge.to_vec();
        clients.push(0);
        assert!(check(&clients).is_err());

        let mut blocks = vec![1];
        blocks.extend(huge);
        blocks.extend([0, 0, 0]);
        assert!(check(&blocks).is_err());

        let mut delete_clients = vec![0];
        delete_clients.extend(huge);
        assert!(check(&delete_clients).is_err());

        let mut delete_ranges = vec![0, 1, 0];
        delete_ranges.extend(huge);
        assert!(check(&delete_ranges).is_err());
    }

    #[test]
    fn rejects_known_malformed_pending_update_without_poisoning_replica() {
        let bad = [1, 1, 0, 0, 0x52, 0, 0, 0, 0, 0];
        let replica = Replica::new();
        assert!(replica.apply_update(&bad, "remote").is_err());
        assert_eq!(replica.text(), "");
        assert!(replica.state().len() > 0);
    }

    #[test]
    fn keeps_yrs_extended_varint_compatibility() {
        use yrs::updates::decoder::Decode;
        // Both counts encode zero using six octets. Yrs accepts this form
        // even though the canonical encoding is simply [0, 0].
        let noncanonical = [
            0x80, 0x80, 0x80, 0x80, 0x80, 0x00,
            0x80, 0x80, 0x80, 0x80, 0x80, 0x00,
        ];
        assert!(Update::decode_v1(&noncanonical).is_ok());
        assert!(decode(&noncanonical).is_ok());
    }

    #[test]
    fn accepts_real_local_yrs_v1_updates_and_rejects_truncation() {
        let replica = Replica::new();
        let seed = replica.state();
        assert!(check(&seed).is_ok());
        let long = "a".repeat(32_768);
        let update = replica.apply_edits(&[TextEdit { from: 0, to: 0, insert: long }], "local");
        assert!(check(&update).is_ok());
        assert!(check(&replica.state()).is_ok());
        assert!(check(&[1]).is_err());
        assert!(check(&[]).is_err());
    }
}
