use super::*;
use aquilum_core::sync::{NativeRecoveryOverview, NativeRecoveryDecision};
mod yandex;
pub(super) use yandex::yandex_setup;

/// User-controlled *documentation* endpoints. Never use arbitrary input as a
/// link target, endpoint or credential provider inside this setup surface.
pub(super) const YANDEX_CONSOLE: &str = "https://console.yandex.cloud/";
pub(super) const YANDEX_ACCOUNT_DOCS: &str = "https://yandex.cloud/ru/docs/iam/operations/sa/create";
pub(super) const YANDEX_KEY_DOCS: &str = "https://yandex.cloud/ru/docs/iam/operations/sa/create-access-key";
pub(super) const YANDEX_BUCKET_DOCS: &str = "https://yandex.cloud/ru/docs/storage/operations/buckets/create";
pub(super) const YANDEX_AWS_PROFILE_DOCS: &str = "https://yandex.cloud/ru/docs/storage/quickstart/quickstart-aws-cli";

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
    // S3-E2E-01: separate cloud-VaultId visibility gate. The ordinary
    // local-workspace dashboard must NOT equate workspace UUID and cloud ID.
    // This preview is strictly observational: no action buttons, receipts,
    // File Gate grants, S3 ACK or baseline promotion.
    #[cfg(feature = "s3-e2e01-review-preview")]
    if let Some(s3_status) = ctx.s3_bound_publications.as_ref() {
        let (value, tone) = match s3_status {
            NativeRecoveryOverview::JournalNotConfigured =>
                (t("settings.sync.s3BoundNotConfigured"), Tone::Muted),
            NativeRecoveryOverview::NoLocalPublicationClaims =>
                (t("settings.sync.s3BoundNoClaims"), Tone::Secondary),
            NativeRecoveryOverview::RequiresReview { .. } =>
                (t("settings.sync.s3BoundHold"), Tone::Danger),
            NativeRecoveryOverview::InspectionUnavailable =>
                (t("settings.sync.s3BoundUnavailable"), Tone::Danger),
        };
        rows.push(info(
            t("settings.sync.s3BoundTitle"),
            Some(t("settings.sync.s3BoundHint")),
            Control::Status { text: value, tone },
        ));
        if let NativeRecoveryOverview::RequiresReview { total_claims, examples } =
            s3_status
        {
            rows.push(info(
                t("settings.sync.s3BoundCount"),
                Some(t("settings.sync.s3BoundNoAck")),
                Control::Status {
                    text: total_claims.to_string(),
                    tone: Tone::Danger,
                },
            ));
            rows.extend(examples.iter().map(|case| info(
                case.historical_path.clone(),
                Some(t("settings.sync.s3BoundHistoricalPath")),
                Control::Status {
                    text: t("settings.sync.s3BoundHold"),
                    tone: Tone::Danger,
                },
            )));
        }
    }
    // 3.13 Dropbox integration visibility. This is deliberately read-only:
    // OAuth onboarding is still a standalone developer probe, not a native
    // UI action, and must not be represented as a connected account.
    rows.push(info(
        "Dropbox (3.13 preview)".to_owned(),
        Some("Read-only connector exists as a separate QA probe; native authorization and synchronization are not enabled.".to_owned()),
        Control::Status {
            text: "DISCONNECTED / SYNC_DISABLED".to_owned(),
            tone: Tone::Secondary,
        },
    ));
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
