//! UTF-16 scalar-boundary checks before passing edits to Yrs 0.28.0.
//! Upstream #714: split-surrogate positions can mutate the wrong character or panic.
use super::{failure, ReplicaError, TextEdit};

pub(super) fn validate_utf16_edits(source: &str, edits: &[TextEdit]) -> Result<(), ReplicaError> {
    if edits.is_empty() {
        return Ok(());
    }

    let mut endpoints = Vec::with_capacity(edits.len().saturating_mul(2));
    let mut previous_end = 0;
    for edit in edits {
        if edit.from < previous_end || edit.from > edit.to {
            return Err(failure("overlapping or descending UTF-16 edits"));
        }
        if edit.to > u32::MAX as usize {
            return Err(failure("UTF-16 edit offset exceeds Yrs limit"));
        }
        endpoints.extend([edit.from, edit.to]);
        previous_end = edit.to;
    }

    endpoints.sort_unstable();
    endpoints.dedup();
    let mut position = 0;
    let mut next_endpoint = 0;
    for character in source.chars() {
        while next_endpoint < endpoints.len() && endpoints[next_endpoint] == position {
            next_endpoint += 1;
        }
        position += character.len_utf16();
        if next_endpoint < endpoints.len() && endpoints[next_endpoint] < position {
            return Err(failure("UTF-16 edit splits a surrogate pair"));
        }
    }
    while next_endpoint < endpoints.len() && endpoints[next_endpoint] == position {
        next_endpoint += 1;
    }
    if next_endpoint != endpoints.len() {
        return Err(failure("UTF-16 edit extends beyond the document"));
    }
    Ok(())
}
