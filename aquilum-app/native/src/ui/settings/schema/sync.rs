use super::*;
use aquilum_core::sync::{NativeRecoveryOverview, NativeRecoveryDecision};
use aquilum_core::settings::models::YandexS3Onboarding;

/// User-controlled *documentation* endpoints. Never use arbitrary input as a
/// link target, endpoint or credential provider inside this setup surface.
pub(super) const YANDEX_CONSOLE: &str = "https://console.yandex.cloud/";
pub(super) const YANDEX_ACCOUNT_DOCS: &str = "https://yandex.cloud/ru/docs/iam/operations/sa/create";
pub(super) const YANDEX_KEY_DOCS: &str = "https://yandex.cloud/ru/docs/iam/operations/sa/create-access-key";
pub(super) const YANDEX_BUCKET_DOCS: &str = "https://yandex.cloud/ru/docs/storage/operations/buckets/create";
pub(super) const YANDEX_AWS_PROFILE_DOCS: &str = "https://yandex.cloud/ru/docs/storage/quickstart/quickstart-aws-cli";

/// YC-S3-T01, UI stage only. A validated draft is NOT authorization, remote
/// account identity proof, a live S3 HEAD, or permission to start syncing.
/// Explicitly no field accepts a secret: native TextEditor fields are plaintext.
pub(super) fn yandex_setup(config: &AppConfig) -> Block {
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

/// A read-only native safety surface. In particular, DO NOT add generic
/// Action::Restore, cleanup, "retry sync" or stage-delete controls here:
/// the current M1d journal has no cross-process provenance/durability receipt.
pub(super) fn sync_recovery(ctx: &Context) -> Vec<Block> {
    let (label, description, value, tone) = match ctx.sync_recovery.as_ref() {
        None => (
            t("settings.sync.noVault"),
            t("settings.sync.noVaultHint"),
            t("settings.sync.unavailable"),
            Tone::Muted,
        ),
        Some(NativeRecoveryOverview::JournalNotConfigured) => (
            t("settings.sync.notConfigured"),
            t("settings.sync.notConfiguredHint"),
            t("settings.sync.offline"),
            Tone::Secondary,
        ),
        Some(NativeRecoveryOverview::NoLocalPublicationClaims) => (
            t("settings.sync.noClaims"),
            t("settings.sync.noClaimsHint"),
            t("settings.sync.notProof"),
            Tone::Secondary,
        ),
        Some(NativeRecoveryOverview::RequiresReview { total_claims, .. }) => (
            t("settings.sync.pending"),
            t_with("settings.sync.pendingHint", &[("count", &total_claims.to_string())]),
            t("settings.sync.reviewRequired"),
            Tone::Danger,
        ),
        Some(NativeRecoveryOverview::InspectionUnavailable) => (
            t("settings.sync.failed"),
            t("settings.sync.failedHint"),
            t("settings.sync.blocked"),
            Tone::Danger,
        ),
    };
    let mut rows = vec![
        info(label, Some(description),
            Control::Status { text: value, tone }),
    ];
    if let Some(NativeRecoveryOverview::RequiresReview { examples, total_claims }) =
        ctx.sync_recovery.as_ref()
    {
        rows.extend(examples.iter().take(5).map(|example| {
            info(
                example.historical_path.clone(),
                Some(t("settings.sync.historicalPathHint")),
                Control::Buttons(vec![
                    Button::new(t("settings.sync.inspect"),
                        Action::InspectRecovery(example.operation_id.clone())),
                ]),
            )
        }));
        if *total_claims > examples.len() {
            rows.push(info(
                t("settings.sync.more"),
                Some(t_with("settings.sync.moreHint",
                    &[("count", &(*total_claims - examples.len()).to_string())])),
                Control::Status {
                    text: t("settings.sync.hold"),
                    tone: Tone::Secondary,
                },
            ));
        }
        if let Some(preview) = ctx.sync_selected.as_ref().filter(|preview| {
            examples.iter().any(|entry| entry.operation_id == preview.operation_id
                && entry.historical_path == preview.historical_path)
        }) {
            rows.push(info(
                t("settings.sync.selected"),
                Some(preview.historical_path.clone()),
                Control::Status { text: preview.operation_id.clone(), tone: Tone::Secondary },
            ));
            rows.push(info(
                t("settings.sync.assessment"),
                Some(t("settings.sync.assessmentHint")),
                Control::Status { text: preview.assessment.clone(), tone: Tone::Danger },
            ));
            rows.push(info(
                t("settings.sync.fingerprint"),
                Some(t("settings.sync.fingerprintHint")),
                Control::Status { text: preview.archive_fingerprint.clone(), tone: Tone::Secondary },
            ));
            rows.push(info(
                t("settings.sync.reviewCount"),
                Some(t("settings.sync.reviewCountHint")),
                Control::Status { text: preview.review_count.to_string(), tone: Tone::Secondary },
            ));
            rows.push(info(
                t("settings.sync.reviewAction"),
                Some(t("settings.sync.reviewActionHint")),
                Control::Buttons(vec![
                    Button::new(t("settings.sync.keepBlocked"),
                        Action::RequestRecoveryReview(NativeRecoveryDecision::KeepBlocked)),
                    Button::new(t("settings.sync.escalate"),
                        Action::RequestRecoveryReview(NativeRecoveryDecision::EscalateManualReview)),
                ]),
            ));
        }
    }
    if let Some(notice) = &ctx.sync_notice {
        rows.push(info(t("settings.sync.reviewResult"), None,
            Control::Status { text: notice.clone(), tone: Tone::Secondary }));
    }
    rows.push(info(
        t("settings.sync.retention"),
        Some(t("settings.sync.retentionHint")),
        Control::Status {
            text: t("settings.sync.retentionStatus"),
            tone: Tone::Secondary,
        },
    ));
    rows.push(info(
        t("settings.sync.safety"),
        Some(t("settings.sync.safetyHint")),
        Control::Status {
            text: t("settings.sync.noWriteAccess"),
            tone: Tone::Muted,
        },
    ));
    vec![Block { title: t("settings.sync.title"), rows }]
}
