//! Fuzz regressions from linebender/vello#2005: invalid culling row ranges
//! must not index outside the viewport bitmask.

use super::CulledWindings;
use std::vec::Vec;

#[test]
fn vello_culling_range_zero_height_and_huge_rows_are_noops() {
    let mut winding = CulledWindings::new(0);
    winding.mark_row_active(0);
    winding.mark_row_active(usize::MAX);
    winding.mark_row_range_active(0, 1);
    winding.mark_row_range_active(0, usize::MAX);
    winding.mark_row_range_active(usize::MAX, usize::MAX);
    winding.for_active_rows_in_range(0, usize::MAX, |_| {
        panic!("zero-height viewport must have no active rows");
    });
    assert!(winding.active.is_empty());
}

#[test]
fn vello_culling_range_invalid_limits_do_not_lose_valid_row_bits() {
    let mut winding = CulledWindings::new(5); // Two 4-pixel tile rows.
    winding.mark_row_active(usize::MAX);
    winding.mark_row_range_active(usize::MAX - 1, usize::MAX);
    assert_eq!(winding.active, [0]);
    winding.mark_row_range_active(0, usize::MAX);
    let mut touched = Vec::new();
    winding.for_active_rows_in_range(0, usize::MAX, |r| touched.push(r));
    assert_eq!(touched, [0, 1]);
    winding.mark_row_active(10_000);
    assert_eq!(winding.active, [3]);
    let mut only_last = Vec::new();
    winding.for_active_rows_in_range(1, 1_000_000_000, |r| only_last.push(r));
    assert_eq!(only_last, [1]);
    winding.for_active_rows_in_range(usize::MAX, usize::MAX, |_| panic!("invalid range"));
}

#[test]
fn vello_culling_range_multiword_matches_valid_viewport_oracle() {
    let mut winding = CulledWindings::new(281); // 71 rows, last word partially populated.
    winding.mark_row_range_active(11, 40);
    winding.mark_row_active(60);
    winding.mark_row_range_active(70, usize::MAX);
    winding.mark_row_range_active(100, usize::MAX);
    let mut seen = Vec::new();
    winding.for_active_rows_in_range(0, usize::MAX, |r| seen.push(r));
    let expected: Vec<usize> = (11..40).chain([60, 70]).collect();
    assert_eq!(seen, expected);
    let mut cropped = Vec::new();
    winding.for_active_rows_in_range(30, 65, |r| cropped.push(r));
    let expected_crop: Vec<usize> = (30..40).chain([60]).collect();
    assert_eq!(cropped, expected_crop);
}

#[test]
fn vello_culling_range_missing_mask_words_never_panic() {
    // The bitmask is public; keep exposed traversal operations fail-safe if
    // a caller supplies an inconsistent mask after construction.
    let mut winding = CulledWindings::new(20);
    winding.active.clear();
    winding.mark_row_active(0);
    winding.mark_row_range_active(0, 5);
    winding.for_active_rows_in_range(0, 5, |_| panic!("missing mask words"));
    assert!(winding.active.is_empty());
}
