//! E3i experimental, **offline only** Native owner comparison UI state.
//! Never serialize this state into AppConfig, local documents or logs.
use aquilum_core::sync::OwnerReviewPreviewAttempt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewState {
    AwaitingIndependentOwner,
    MatchedOfflineOnly,
    Rejected,
}

pub struct OwnerReview {
    pub attempt: OwnerReviewPreviewAttempt,
    pub typed: String,
    pub state: ReviewState,
}

#[derive(Clone)]
pub struct OwnerReviewView {
    pub untrusted_bucket: String,
    pub challenge_hex: Option<String>,
    pub state: ReviewState,
}

impl OwnerReview {
    pub fn new(attempt: OwnerReviewPreviewAttempt) -> Self {
        Self {
            attempt,
            typed: String::new(),
            state: ReviewState::AwaitingIndependentOwner,
        }
    }
    pub fn view(&self) -> OwnerReviewView {
        OwnerReviewView {
            untrusted_bucket: self.attempt.untrusted_bucket().to_owned(),
            challenge_hex: (!self.attempt.spent()).then(|| self.attempt.challenge_hex().to_owned()),
            state: self.state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::sync::MAX_OWNER_REVIEW_PROPOSAL_BYTES;

    #[test]
    fn ephemeral_state_starts_without_any_stored_owner_code() {
        let candidate = serde_json::json!({
            "schema": 1, "provider": "yandex-object-storage",
            "endpoint": "https://storage.yandexcloud.net",
            "region": "ru-central1", "force_path_style": true,
            "bucket": "preview-bucket", "prefix": "akula/test",
            "vault_id": uuid::Uuid::new_v4().to_string(),
            "genesis_root_blake3": "a".repeat(64),
            "genesis_policy_blake3": "b".repeat(64),
            "owner_keys_hex": ["1".repeat(64)], "owner_quorum": 1
        });
        let bytes = serde_json::to_vec(&candidate).unwrap();
        assert!(bytes.len() < MAX_OWNER_REVIEW_PROPOSAL_BYTES);
        let attempt = OwnerReviewPreviewAttempt::import_untrusted(&bytes).unwrap();
        let review = OwnerReview::new(attempt);
        assert!(review.typed.is_empty());
        assert_eq!(review.state, ReviewState::AwaitingIndependentOwner);
        assert_eq!(review.view().challenge_hex.unwrap().len(), 64);
    }
}
