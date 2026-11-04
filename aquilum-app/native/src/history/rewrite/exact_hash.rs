//! Bind a Restore to the raw bytes that the editor last confirmed saved.
//! Matching is repeated by File Gate CAS after the staging write, not only here.

use aquilum_core::files::error::FileCommandError;

pub(super) fn require_expected(expected: Option<&str>, actual: &str) -> Result<(), FileCommandError> {
    if let Some(expected) = expected.filter(|hash| *hash != actual) {
        return Err(FileCommandError::Conflict {
            expected_hash: expected.to_owned(),
            actual_hash: Some(actual.to_owned()),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_editor_digest_is_a_visible_conflict() {
        let error = require_expected(Some("old"), "new").unwrap_err();
        assert!(matches!(
            error,
            FileCommandError::Conflict { expected_hash, actual_hash }
                if expected_hash == "old" && actual_hash.as_deref() == Some("new")
        ));
    }

    #[test]
    fn matching_digest_and_revert_without_bound_digest_are_allowed() {
        assert!(require_expected(Some("same"), "same").is_ok());
        assert!(require_expected(None, "any").is_ok());
    }
}
