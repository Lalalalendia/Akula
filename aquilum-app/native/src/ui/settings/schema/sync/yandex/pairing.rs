//! Compact, honest S3 pairing controls. Every action is a real native
//! background operation, never a decorative UI-only "connected" badge.
//! Only the explicitly opted-in Windows beta enables these buttons.

use super::*;
use crate::s3_pair_session::PairStatus;

pub(super) fn rows(ctx: &Context, setup_ready: bool) -> Vec<Row> {
    let available = !ctx.s3_pair.busy;
    let status = ctx.s3_pair.status;
    let has_profile = ctx.s3_pair.configured;
    let (message, tone) = match status {
        PairStatus::Idle if has_profile =>
            (t("settings.yandex.pairConfigured"), Tone::Secondary),
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
    if has_profile || matches!(status, PairStatus::Configured | PairStatus::Imported |
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

/// Production-shaped compact layout for the opt-in Windows pairing beta.
/// Users see the provider preset, bucket, protected key status and two
/// primary actions, not developer-facing AWS CLI/tutorial steps.
pub(super) fn compact(
    config: &AppConfig,
    ctx: &Context,
    view: super::controls::ViewState,
) -> Block {
    let draft = &config.s3_onboarding;
    let mut fields = vec![
        info(
            t("settings.yandex.pairProvider"),
            Some(t("settings.yandex.pairProviderHint")),
            Control::Status {
                text: "Yandex Object Storage".into(),
                tone: Tone::Secondary,
            },
        ),
        row(
            t("settings.yandex.bucket"),
            Some(t("settings.yandex.bucketHint")),
            Control::Text { value: draft.bucket.clone(), commit: true },
            |c, v| {
                let name = text(v);
                if YandexS3Onboarding::valid_bucket(&name) {
                    c.s3_onboarding.bucket = name;
                }
            },
        ),
        info(
            t("settings.yandex.keyStatus"),
            Some(t("settings.yandex.credentialSafetyHint")),
            Control::Status { text: view.key_status, tone: view.key_tone },
        ),
        info(
            t("settings.yandex.manageKeys"),
            Some(t("settings.yandex.manageKeysHint")),
            Control::Buttons(vec![
                Button::new(t("settings.yandex.addKeys"), Action::StoreYandexKeys)
                                        .enabled(view.supported && !ctx.s3_pair.busy),
                Button::new(t("settings.yandex.removeKeys"), Action::RemoveYandexKeys)
                    .enabled(view.supported && view.bucket_ready
                        && ctx.yandex_keys == Some(true) && !ctx.s3_pair.busy),
            ]),
        ),
    ];
    fields.extend(rows(ctx, view.supported && view.bucket_ready));
    if let Some(notice) = &ctx.yandex_notice {
        fields.push(info(t("settings.yandex.keyActionResult"), None,
            Control::Status { text: notice.clone(), tone: Tone::Secondary }));
    }
    Block { title: t("settings.yandex.pairEntry"), rows: fields }
}
