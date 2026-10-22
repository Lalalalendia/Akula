use super::preview_dimensions_allowed;

#[test]
fn native_pixel_budget_is_separate_from_compressed_byte_budget() {
    assert!(preview_dimensions_allowed((6_000, 4_000)));
    assert!(preview_dimensions_allowed((1, 24_000_000)));
    assert!(!preview_dimensions_allowed((6_001, 4_000)));
    assert!(!preview_dimensions_allowed((65_000, 65_000)));
    assert!(!preview_dimensions_allowed((0, 1)));
    assert!(!preview_dimensions_allowed((1, 0)));
}
