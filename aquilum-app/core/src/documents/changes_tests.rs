use super::*;
use crate::documents::merge::apply_edits;
use serde_json::json;

const DOC: &str = "😀 мир\nвторой";

#[test]
fn reads_a_change_set_serialized_by_codemirror() {
    let changes = json!([[2, "A"], 4, [0, "", "новая", "строка"], 2, [2], 3]);
    let edits = edits_from_json(&changes, 13).expect("valid change set");
    assert_eq!(apply_edits(DOC, &edits),"A мир\nновая\nстрока\nврой");
}

#[test]
fn writes_back_the_same_change_set() {
    let changes = json!([[2, "A"], 4, [0, "", "новая", "строка"], 2, [2], 3]);
    let edits = edits_from_json(&changes, 13).expect("valid change set");
    assert_eq!(edits_to_json(&edits, 13), changes);
}

#[test]
fn an_empty_change_set_keeps_the_whole_document() {
    assert_eq!(edits_to_json(&[], 13), json!([13]));
    assert!(edits_from_json(&json!([13]), 13).expect("valid").is_empty());
    assert_eq!(edits_to_json(&[], 0), json!([]));
}

#[test]
fn refuses_a_change_set_made_for_another_document_length() {
    assert!(edits_from_json(&json!([12]), 13).is_err());
    assert!(edits_from_json(&json!([[20]]), 13).is_err());
}

#[test]
fn refuses_malformed_sections() {
    assert!(edits_from_json(&json!({}), 0).is_err());
    assert!(edits_from_json(&json!([[]]), 0).is_err());
    assert!(edits_from_json(&json!([["x"]]), 0).is_err());
    assert!(edits_from_json(&json!([[0, 5]]), 0).is_err());
}

#[test]
fn computes_the_length_after_the_edits_in_utf16_units() {
    let edits = vec![TextEdit { from: 0, to: 2, insert: "🎉🎉".into() }];
    assert_eq!(length_after(&edits, 13), 15);
}

#[test]
fn rejects_section_lengths_that_overflow_before_covering_the_document() {
    for changes in [
        json!([u64::MAX, 1, 3]),
        json!([1, [u64::MAX], 3]),
        json!([[u64::MAX], 1, 3]),
    ] {
        assert!(edits_from_json(&changes, 3).is_err(), "{changes}");
    }
}

#[test]
fn accepts_maximum_positions_without_overflow_when_coverage_is_valid() {
    assert!(edits_from_json(&json!([usize::MAX]), usize::MAX).unwrap().is_empty());
    let edits = edits_from_json(&json!([[usize::MAX]]), usize::MAX).unwrap();
    assert_eq!((edits[0].from, edits[0].to), (0, usize::MAX));
}
