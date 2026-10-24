//! Allocation preflight for the *outer header* of a Yrs v1 Update.
//!
//! Yrs 0.28.0 reserves memory from declared client and block counts before it
//! establishes that the wire payload contains enough bytes (upstream #675).
//! This checks only provable length invariants; it is NOT a full Yrs validator.
//! Raw updates from untrusted peers still require an upstream decoder fix.
use super::{failure, ReplicaError};

fn varuint(bytes: &[u8], cursor: &mut usize, max_octets: usize) -> Option<u64> {
    let mut number = 0u64;
    for index in 0..max_octets {
        let octet = *bytes.get(*cursor)?;
        *cursor += 1;
        let shift = index * 7;
        let chunk = u64::from(octet & 0x7f);
        if chunk > u64::MAX >> shift {
            return None;
        }
        number |= chunk << shift;
        if octet & 0x80 == 0 {
            return Some(number);
        }
    }
    None
}

fn varuint32(bytes: &[u8], cursor: &mut usize) -> Option<u32> {
    u32::try_from(varuint(bytes, cursor, 5)?).ok()
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
        varuint(update, &mut cursor, 10)
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
            varuint(update, &mut cursor, 10)
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
