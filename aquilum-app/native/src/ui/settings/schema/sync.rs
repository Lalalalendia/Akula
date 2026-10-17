use super::*;
use aquilum_core::sync::NativeRecoveryOverview;

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
    if let Some(NativeRecoveryOverview::RequiresReview { example_paths, total_claims }) =
        ctx.sync_recovery.as_ref()
    {
        rows.extend(example_paths.iter().take(5).map(|path| {
            info(
                path.clone(),
                Some(t("settings.sync.historicalPathHint")),
                Control::Status {
                    text: t("settings.sync.hold"),
                    tone: Tone::Secondary,
                },
            )
        }));
        if *total_claims > example_paths.len() {
            rows.push(info(
                t("settings.sync.more"),
                Some(t_with("settings.sync.moreHint",
                    &[("count", &(*total_claims - example_paths.len()).to_string())])),
                Control::Status {
                    text: t("settings.sync.hold"),
                    tone: Tone::Secondary,
                },
            ));
        }
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
