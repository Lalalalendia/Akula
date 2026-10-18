//! FED-INVENTORY-01: read-only peer-inventory candidate discovery.
//!
//! NOT a clone of iroh-docs' XOR range protocol. This is a deterministic
//! BLAKE3 prefix-tree comparison to test whether sparse differences can avoid
//! full inventory exchange. A match is a cryptographic heuristic; every
//! candidate still needs Akula VaultId/grant/signature/DAG verification.
//! Never turn absence into deletion or advance a sync baseline here.

use std::collections::BTreeMap;

pub const LEAF_ITEMS: usize = 32;
const NODE_WIRE_BYTES: u64 = 2 * (32 + 8); // two digests and 64-bit counts
const LEAF_HEADER_BYTES: u64 = 2 * 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Revision {
    pub id: [u8; 32],
    pub content_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryError {
    DuplicateRevision,
    ForeignVault,
    NoGrant,
    VisitBudgetExceeded,
}

#[derive(Clone, Debug)]
pub struct Inventory {
    vault: [u8; 16],
    permitted: bool,
    entries: Vec<Revision>,
}

impl Inventory {
    /// Grants here are FIXTURE BOOLEANS, not real Akula credentials.
    pub fn fixture(vault: [u8; 16], permitted: bool, mut entries: Vec<Revision>)
        -> Result<Self, InventoryError>
    {
        entries.sort_by_key(|item| item.id);
        if entries.windows(2).any(|w| w[0].id == w[1].id) {
            return Err(InventoryError::DuplicateRevision);
        }
        Ok(Self { vault, permitted, entries })
    }

    pub fn entries(&self) -> &[Revision] { &self.entries }

    /// Public scope for callers to bind a signed event to the same inventory.
    /// This is a fixture byte-array, NOT proof of a genuine Akula VaultId grant.
    pub fn vault_scope(&self) -> [u8; 16] { self.vault }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Stats {
    pub nodes_compared: u64,
    pub leaf_exchanges: u64,
    pub estimated_bytes: u64,
    pub full_list_bytes: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CandidateDiff {
    /// Exact bytes are present in B but absent in A.
    pub missing_from_a: Vec<Revision>,
    /// Exact bytes are present in A but absent in B.
    pub missing_from_b: Vec<Revision>,
    /// Same RevisionId but conflicting content digest: DO NOT auto-merge.
    pub collisions: Vec<(Revision, Revision)>,
    pub stats: Stats,
}

fn require_scope(a: &Inventory, b: &Inventory) -> Result<(), InventoryError> {
    if !a.permitted || !b.permitted {
        return Err(InventoryError::NoGrant);
    }
    if a.vault != b.vault {
        return Err(InventoryError::ForeignVault);
    }
    Ok(())
}

fn digest(slice: &[Revision]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"akula/fed-inventory/01/range");
    hasher.update(&(slice.len() as u64).to_be_bytes());
    for item in slice {
        hasher.update(&item.id);
        hasher.update(&item.content_digest);
    }
    *hasher.finalize().as_bytes()
}

fn bit_of(id: &[u8; 32], bit: usize) -> bool {
    (id[bit / 8] & (0x80u8 >> (bit % 8))) != 0
}

fn enumerate(a: &[Revision], b: &[Revision], diff: &mut CandidateDiff) {
    let mut i = 0;
    let mut j = 0;
    while i < a.len() || j < b.len() {
        match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) if x.id == y.id => {
                if x.content_digest != y.content_digest {
                    diff.collisions.push((*x, *y));
                }
                i += 1;
                j += 1;
            }
            (Some(x), Some(y)) if x.id < y.id => {
                diff.missing_from_b.push(*x);
                i += 1;
            }
            (Some(_), Some(y)) => {
                diff.missing_from_a.push(*y);
                j += 1;
            }
            (Some(x), None) => {
                diff.missing_from_b.push(*x);
                i += 1;
            }
            (None, Some(y)) => {
                diff.missing_from_a.push(*y);
                j += 1;
            }
            (None, None) => break,
        }
    }
}

fn walk(
    a: &[Revision],
    b: &[Revision],
    bit: usize,
    max_nodes: u64,
    diff: &mut CandidateDiff,
) -> Result<(), InventoryError> {
    if diff.stats.nodes_compared >= max_nodes {
        return Err(InventoryError::VisitBudgetExceeded);
    }
    diff.stats.nodes_compared += 1;
    diff.stats.estimated_bytes += NODE_WIRE_BYTES;
    if digest(a) == digest(b) {
        return Ok(());
    }

    if bit == 256 || a.len().max(b.len()) <= LEAF_ITEMS {
        diff.stats.leaf_exchanges += 1;
        diff.stats.estimated_bytes += LEAF_HEADER_BYTES + (a.len() + b.len()) as u64 * 64;
        enumerate(a, b, diff);
        return Ok(());
    }

    // Lexicographically sorted 32-byte IDs have a stable split by prefix bit.
    // Splits depend on the prefix position, NOT on local population size.
    let ap = a.partition_point(|item| !bit_of(&item.id, bit));
    let bp = b.partition_point(|item| !bit_of(&item.id, bit));
    walk(&a[..ap], &b[..bp], bit + 1, max_nodes, diff)?;
    walk(&a[ap..], &b[bp..], bit + 1, max_nodes, diff)?;
    Ok(())
}

/// Exactly scoped candidate discovery, not a signed proof or a write permit.
/// Returns no partial output if the comparison exhausts its work budget.
pub fn compare(a: &Inventory, b: &Inventory, max_nodes: u64)
    -> Result<CandidateDiff, InventoryError>
{
    require_scope(a, b)?;
    let mut diff = CandidateDiff::default();
    diff.stats.full_list_bytes = LEAF_HEADER_BYTES + (a.entries.len() + b.entries.len()) as u64 * 64;
    walk(&a.entries, &b.entries, 0, max_nodes, &mut diff)?;
    // The same differing entries can be encountered in disjoint prefixes,
    // but the final candidate output order is normalized for stable tests.
    diff.missing_from_a.sort_by_key(|x| x.id);
    diff.missing_from_b.sort_by_key(|x| x.id);
    diff.collisions.sort_by_key(|(x, _)| x.id);
    Ok(diff)
}

/// Independently computed full-list baseline. No tree, hashing or range hints.
pub fn full_compare(a: &Inventory, b: &Inventory)
    -> Result<CandidateDiff, InventoryError>
{
    require_scope(a, b)?;
    let left: BTreeMap<_, _> = a.entries.iter().map(|x| (x.id, *x)).collect();
    let right: BTreeMap<_, _> = b.entries.iter().map(|x| (x.id, *x)).collect();
    let mut result = CandidateDiff::default();
    for (id, item) in &left {
        match right.get(id) {
            None => result.missing_from_b.push(*item),
            Some(other) if item.content_digest != other.content_digest =>
                result.collisions.push((*item, *other)),
            Some(_) => {},
        }
    }
    for (id, item) in &right {
        if !left.contains_key(id) {
            result.missing_from_a.push(*item);
        }
    }
    Ok(result)
}

/// Fixed, repeatable fixture. Not a real RevisionId signer or grant.
pub fn sample_revision(index: u64) -> Revision {
    let number = index.to_be_bytes();
    let mut hash = blake3::Hasher::new();
    hash.update(b"fed-inventory-01/revision-id");
    hash.update(&number);
    let id = *hash.finalize().as_bytes();
    let mut hash = blake3::Hasher::new();
    hash.update(b"fed-inventory-01/payload");
    hash.update(&number);
    Revision { id, content_digest: *hash.finalize().as_bytes() }
}

pub const TEST_VAULT: [u8; 16] = [42; 16];

#[cfg(test)]
mod tests {
    use super::*;

    fn check(a: Vec<Revision>, b: Vec<Revision>) -> CandidateDiff {
        let a = Inventory::fixture(TEST_VAULT, true, a).unwrap();
        let b = Inventory::fixture(TEST_VAULT, true, b).unwrap();
        let got = compare(&a, &b, 100_000).unwrap();
        let expected = full_compare(&a, &b).unwrap();
        assert_eq!(got.missing_from_a, expected.missing_from_a);
        assert_eq!(got.missing_from_b, expected.missing_from_b);
        assert_eq!(got.collisions, expected.collisions);
        got
    }

    #[test]
    fn empty_and_singleton_inventories_are_exact() {
        check(vec![], vec![]);
        check(vec![sample_revision(7)], vec![]);
        check(vec![], vec![sample_revision(8)]);
        check(vec![sample_revision(7)], vec![sample_revision(8)]);
    }

    #[test]
    fn nonuniform_prefixes_keep_exact_revision_boundaries() {
        // Concentrate 257 distinct entries into a common first 28 bytes.
        // Prefix splits must be positional, not based on per-peer medians.
        let all: Vec<_> = (0..257u64).map(|i| {
            let mut id = [0u8; 32];
            id[28..].copy_from_slice(&(i as u32).to_be_bytes());
            Revision {
                id,
                content_digest: sample_revision(i).content_digest,
            }
        }).collect();
        let fewer: Vec<_> = all.iter().copied().filter(|x| x.id != all[211].id).collect();
        let got = check(all, fewer);
        assert_eq!(got.missing_from_b.len(), 1);
        assert_eq!(got.missing_from_b[0].id[28..], 211u32.to_be_bytes());
    }

    #[test]
    fn deterministic_mixed_set_matrix_matches_independent_baseline() {
        let entries: Vec<_> = (0..1200).map(sample_revision).collect();
        for seed in 0u64..64 {
            let a: Vec<_> = entries.iter().copied()
                .filter(|r| ((r.id[0] as u64 + seed * 19) % 11) >= (seed % 5))
                .collect();
            let mut b: Vec<_> = entries.iter().copied()
                .filter(|r| ((r.id[1] as u64 + seed * 7) % 13) >= (seed % 7))
                .collect();
            if seed % 3 == 0 {
                let id = sample_revision(999);
                if let Some(v) = b.iter_mut().find(|r| r.id == id.id) {
                    v.content_digest[0] ^= 0x55;
                }
            }
            let _ = check(a, b);
        }
    }

    #[test]
    fn exhaustion_after_partial_leaf_work_is_all_or_nothing() {
        let a = Inventory::fixture(
            TEST_VAULT,
            true,
            (0..10_000).map(sample_revision).collect(),
        ).unwrap();
        let b = Inventory::fixture(
            TEST_VAULT,
            true,
            (1..10_000).map(sample_revision).collect(),
        ).unwrap();
        assert_eq!(compare(&a, &b, 10), Err(InventoryError::VisitBudgetExceeded));
    }

    #[test]
    fn identical_inventories_exchange_just_one_digest_pair() {
        let a: Vec<_> = (0..10_000).map(sample_revision).collect();
        let got = check(a.clone(), a);
        assert!(got.missing_from_a.is_empty() && got.missing_from_b.is_empty());
        assert_eq!(got.stats.nodes_compared, 1);
        assert_eq!(got.stats.estimated_bytes, NODE_WIRE_BYTES);
    }

    #[test]
    fn sparse_difference_saves_bytes_without_losing_a_revision() {
        let a: Vec<_> = (0..10_000).map(sample_revision).collect();
        let mut b = a.clone();
        b.retain(|item| item.id != sample_revision(1337).id);
        let got = check(a, b);
        assert_eq!(got.missing_from_b.len(), 1);
        assert_eq!(got.missing_from_b[0], sample_revision(1337));
        assert!(got.stats.estimated_bytes < got.stats.full_list_bytes / 5);
    }

    #[test]
    fn hundred_missing_and_fifty_new_are_accounted_for() {
        let a: Vec<_> = (0..10_000).map(sample_revision).collect();
        let mut b: Vec<_> = (100..10_000).map(sample_revision).collect();
        b.extend((10_000..10_050).map(sample_revision));
        let got = check(a, b);
        assert_eq!(got.missing_from_b.len(), 100);
        assert_eq!(got.missing_from_a.len(), 50);
    }

    #[test]
    fn half_missing_and_totally_disjoint_do_not_claim_free_speedups() {
        let a: Vec<_> = (0..10_000).map(sample_revision).collect();
        let b: Vec<_> = (0..10_000).filter(|i| i % 2 == 0).map(sample_revision).collect();
        assert_eq!(check(a.clone(), b).missing_from_b.len(), 5_000);
        let b: Vec<_> = (10_000..20_000).map(sample_revision).collect();
        let got = check(a, b);
        assert_eq!(got.missing_from_a.len(), 10_000);
        assert_eq!(got.missing_from_b.len(), 10_000);
    }

    #[test]
    fn same_revision_id_with_changed_bytes_is_a_conflict() {
        let a = sample_revision(1);
        let mut b = a;
        b.content_digest = [8; 32];
        let got = check(vec![a], vec![b]);
        assert_eq!(got.collisions, vec![(a, b)]);
        assert!(got.missing_from_a.is_empty());
    }

    #[test]
    fn order_is_canonical_and_duplicate_revision_ids_fail_closed() {
        let mut a: Vec<_> = (0..1000).map(sample_revision).collect();
        let b = a.clone();
        a.reverse();
        let got = check(a, b);
        assert_eq!(got.stats.nodes_compared, 1);
        let id = sample_revision(3);
        assert_eq!(
            Inventory::fixture(TEST_VAULT, true, vec![id, id]).unwrap_err(),
            InventoryError::DuplicateRevision
        );
    }

    #[test]
    fn vault_mismatch_and_missing_permission_fail_closed() {
        let a = Inventory::fixture(TEST_VAULT, true, vec![sample_revision(1)]).unwrap();
        let b = Inventory::fixture([13;16], true, vec![sample_revision(1)]).unwrap();
        assert_eq!(compare(&a, &b, 100), Err(InventoryError::ForeignVault));
        let b = Inventory::fixture(TEST_VAULT, false, vec![sample_revision(1)]).unwrap();
        assert_eq!(compare(&a, &b, 100), Err(InventoryError::NoGrant));
    }

    #[test]
    fn visit_budget_rejects_instead_of_returning_partial_candidates() {
        let a = Inventory::fixture(TEST_VAULT, true, (0..1000).map(sample_revision).collect()).unwrap();
        let b = Inventory::fixture(TEST_VAULT, true, vec![]).unwrap();
        assert_eq!(compare(&a, &b, 1), Err(InventoryError::VisitBudgetExceeded));
    }
}
