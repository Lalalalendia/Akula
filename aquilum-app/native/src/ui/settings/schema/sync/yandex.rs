use super::*;
use aquilum_core::settings::models::YandexS3Onboarding;

/// YC-S3-T01, UI stage only. A validated draft is NOT authorization, remote
/// account identity proof, a live S3 HEAD, or permission to start syncing.
/// Explicitly no field accepts a secret: native TextEditor fields are plaintext.
pub(crate) fn yandex_setup(config: &AppConfig) -> Block {
    let draft = &config.s3_onboarding;
    let rows = vec![
        info(t("settings.yandex.explanation"), Some(t("settings.yandex.explanationHint")),
            Control::Status { text: t("settings.yandex.inactive"), tone: Tone::Danger }),
        info(t("settings.yandex.stepAccount"), Some(t("settings.yandex.stepAccountHint")),
            Control::Buttons(vec![
                Button::new(t("settings.yandex.openConsole"), Action::OpenUrl(YANDEX_CONSOLE)),
                Button::new(t("settings.yandex.accountDocs"), Action::OpenUrl(YANDEX_ACCOUNT_DOCS)),
            ])),
        info(t("settings.yandex.stepKey"), Some(t("settings.yandex.stepKeyHint")),
            Control::Buttons(vec![
                Button::new(t("settings.yandex.keyDocs"), Action::OpenUrl(YANDEX_KEY_DOCS)),
            ])),
        info(t("settings.yandex.stepBucket"), Some(t("settings.yandex.stepBucketHint")),
            Control::Buttons(vec![
                Button::new(t("settings.yandex.bucketDocs"), Action::OpenUrl(YANDEX_BUCKET_DOCS)),
            ])),
        row(
            t("settings.yandex.bucket"),
            Some(t("settings.yandex.bucketHint")),
            Control::Text { value: draft.bucket.clone(), commit: true },
            |c, v| {
                let value = text(v);
                if YandexS3Onboarding::valid_bucket(&value) {
                    c.s3_onboarding.bucket = value;
                }
            },
        ),
        row(
            t("settings.yandex.profile"),
            Some(t("settings.yandex.profileHint")),
            Control::Text { value: draft.aws_profile_name.clone(), commit: true },
            |c, v| {
                let value = text(v);
                if YandexS3Onboarding::valid_profile_name(&value) {
                    c.s3_onboarding.aws_profile_name = value;
                }
            },
        ),
        info(t("settings.yandex.profileHelp"), Some(t("settings.yandex.profileHelpHint")),
            Control::Buttons(vec![
                Button::new(t("settings.yandex.profileDocs"), Action::OpenUrl(YANDEX_AWS_PROFILE_DOCS)),
            ])),
        info(t("settings.yandex.endpoint"), Some(t("settings.yandex.endpointHint")),
            Control::Status { text: "https://storage.yandexcloud.net".into(), tone: Tone::Secondary }),
        info(t("settings.yandex.region"), Some(t("settings.yandex.regionHint")),
            Control::Status { text: "ru-central1".into(), tone: Tone::Secondary }),
        info(t("settings.yandex.credentialSafety"), Some(t("settings.yandex.credentialSafetyHint")),
            Control::Status { text: t("settings.yandex.neverPasteKeys"), tone: Tone::Danger }),
        info(t("settings.yandex.connectionTest"), Some(t("settings.yandex.connectionTestHint")),
            Control::Buttons(vec![
                // Explicitly unavailable until keychain + read-only probe +
                // Yandex-specific trust bootstrap are implemented and tested.
                Button::new(t("settings.yandex.checkConnection"), Action::OpenUrl(YANDEX_KEY_DOCS)).enabled(false),
            ])),
    ];
    Block { title: t("settings.yandex.title"), rows }
}
