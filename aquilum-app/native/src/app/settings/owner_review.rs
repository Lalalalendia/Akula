//! YC-S3 E3i: user-driven OFFLINE preview of a candidate Vault owner.
//!
//! No remote SDK call, credentials, SQLite enrollment, File Gate or Sync.
//! Never record the proposal pathname, owner code or candidate JSON in
//! AppConfig/logs. All state is dropped on Settings close/section change.
use std::io::Read;
use super::{App, Effect};
use crate::ui::settings::SettingsDialog;
use crate::ui::settings::owner_review::{OwnerReview, ReviewState};
use aquilum_core::sync::{OwnerReviewPreviewAttempt, OwnerReviewPreviewResult,
    MAX_OWNER_REVIEW_PROPOSAL_BYTES};

impl App {
    pub(super) fn import_owner_proposal_preview(&mut self, dialog: &mut SettingsDialog) -> Effect {
        dialog.owner_review = None;
        dialog.owner_review_notice = None;
        let selected = self.workspace.core.settings.get_config().s3_onboarding;
        let bucket = selected.bucket.clone();
        if bucket.is_empty() || !aquilum_core::settings::models::YandexS3Onboarding::valid_bucket(&bucket) {
            dialog.owner_review_notice = Some("settings.yandex.reviewNeedBucket");
            return Effect::Content;
        }
        // FileDialog returns only a pathname; never send it to S3.
        let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"]).pick_file() else {
            return Effect::Content;
        };
        let attempt = (|| {
            let file = std::fs::File::open(&path).ok()?;
            let mut bytes = Vec::with_capacity(MAX_OWNER_REVIEW_PROPOSAL_BYTES + 1);
            file.take((MAX_OWNER_REVIEW_PROPOSAL_BYTES + 1) as u64)
                .read_to_end(&mut bytes).ok()?;
            if bytes.len() > MAX_OWNER_REVIEW_PROPOSAL_BYTES { return None; }
            OwnerReviewPreviewAttempt::import_untrusted(&bytes).ok()
        })();
        let Some(attempt) = attempt else {
            dialog.owner_review_notice = Some("settings.yandex.reviewInvalid");
            return Effect::Content;
        };
        // The user selects the target bucket independently in Settings.
        // Imported proposal cannot silently change it.
        if attempt.untrusted_bucket() != bucket {
            dialog.owner_review_notice = Some("settings.yandex.reviewWrongBucket");
            return Effect::Content;
        }
        dialog.owner_review = Some(OwnerReview::new(attempt, selected.aws_profile_name));
        dialog.owner_review_notice = Some("settings.yandex.reviewLoadedUntrusted");
        Effect::Content
    }

    pub(super) fn compare_owner_fingerprint_preview(&mut self, dialog: &mut SettingsDialog) -> Effect {
        let Some(review) = dialog.owner_review.as_mut() else {
            return Effect::None;
        };
        // Every comparison consumes the preview challenge, even malformed
        // input. This does not satisfy the separate E3e admission challenge.
        let code = std::mem::take(&mut review.typed);
        review.state = match review.attempt.verify_once(&code) {
            Ok(OwnerReviewPreviewResult::FingerprintMatchedOfflineOnly) => ReviewState::MatchedOfflineOnly,
            Err(_) => ReviewState::Rejected,
        };
        dialog.owner_review_notice = Some(match review.state {
            ReviewState::MatchedOfflineOnly => "settings.yandex.reviewMatchedOffline",
            ReviewState::Rejected => "settings.yandex.reviewRejected",
            ReviewState::AwaitingIndependentOwner => "settings.yandex.reviewLoadedUntrusted",
        });
        Effect::Content
    }

    pub(super) fn clear_owner_fingerprint_preview(&mut self, dialog: &mut SettingsDialog) -> Effect {
        dialog.owner_review = None;
        dialog.owner_review_notice = None;
        Effect::Content
    }
}
