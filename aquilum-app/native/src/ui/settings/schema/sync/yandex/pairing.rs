//! Compact, honest S3 pairing controls. Every action is a real native
//! background operation, never a decorative UI-only "connected" badge.
//! Only the explicitly opted-in Windows beta enables these buttons.

use super::*;
use crate::s3_pair_session::PairStatus;

pub(super) fn rows(ctx: &Context, setup_ready: bool) -> Vec<Row> {
    let available = !ctx.s3_pair_busy;
    let status = ctx.s3_pair_status;
    let (message, tone) = match status {
        PairStatus::Idle => (t("settings.yandex.pairInactive"), Tone::Secondary),
        PairStatus::Working => (t("settings.yandex.pairWorking"), Tone::Secondary),
        PairStatus::Configured => (t("settings.yandex.pairConfigured"), Tone::Secondary),
        PairStatus::Exported => (t("settings.yandex.pairExported"), Tone::Secondary),
        PairStatus::Imported => (t("settings.yandex.pairImported"), Tone::Secondary),
        PairStatus::Transferred { received_unconfirmed: 0, needs_review: 0, .. } =>
            (t("settings.yandex.pairSentOnly"), Tone::Secondary),
        PairStatus::Transferred { .. } =>
            (t("settings.yandex.pairHold"), Tone::Danger),
        PairStatus::Failed => (t("settings.yandex.pairFailed"), Tone::Danger),
    };
    let mut output = vec![
        info(
            t("settings.yandex.pairEntry"),
            Some(t("settings.yandex.pairEntryHint")),
            Control::Buttons(vec![
                Button::new(t("settings.yandex.pairConfigure"), Action::ConfigureS3Pair)
                    .enabled(available && setup_ready && ctx.yandex_keys == Some(true)),
                Button::new(t("settings.yandex.pairImport"), Action::ImportS3Pair)
                    .enabled(available),
            ]),
        ),
        info(
            t("settings.yandex.pairProgress"),
            Some(t("settings.yandex.pairProgressHint")),
            Control::Status { text: message, tone },
        ),
    ];
    if matches!(status, PairStatus::Configured | PairStatus::Imported |
        PairStatus::Exported | PairStatus::Transferred { .. })
    {
        output.push(info(
            t("settings.yandex.pairActions"),
            Some(t("settings.yandex.pairActionsHint")),
            Control::Buttons(vec![
                Button::new(t("settings.yandex.pairExport"), Action::ExportS3Pair)
                    .enabled(available),
                Button::new(t("settings.yandex.pairSync"), Action::SyncS3PairOnce)
                    .enabled(available),
            ]),
        ));
    }
    output
}
