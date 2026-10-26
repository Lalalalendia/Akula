use super::*;

pub(super) struct ViewState {
    pub(super) supported: bool,
    pub(super) bucket_ready: bool,
    pub(super) preview: bool,
    pub(super) key_status: String,
    pub(super) key_tone: Tone,
    pub(super) probe_status: String,
    pub(super) probe_tone: Tone,
}

/// UI controls are a separate unit from credential/probe status decisions.
/// Every action remains gated by the preview feature and Windows OS support.
pub(super) fn build(config: &AppConfig, ctx: &Context, view: ViewState) -> Block {
    let draft = &config.s3_onboarding;
    let ViewState {
        supported,
        bucket_ready,
        preview,
        key_status,
        key_tone,
        probe_status,
        probe_tone,
    } = view;
    let mut rows = vec![
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
        info(t("settings.yandex.keyStatus"), Some(t("settings.yandex.keyStatusHint")),
            Control::Status { text: key_status, tone: key_tone }),
        info(t("settings.yandex.manageKeys"), Some(t("settings.yandex.manageKeysHint")),
            Control::Buttons(vec![
                Button::new(t("settings.yandex.addKeys"), Action::StoreYandexKeys)
                    .enabled(supported && bucket_ready),
                Button::new(t("settings.yandex.removeKeys"), Action::RemoveYandexKeys)
                    .enabled(supported && bucket_ready && ctx.yandex_keys == Some(true)),
            ])),
        info(t("settings.yandex.connectionTest"), Some(t("settings.yandex.connectionTestHint")),
            Control::Buttons(vec![
                Button::new(
                    if preview { t("settings.yandex.checkConnectionPreview") }
                    else { t("settings.yandex.checkConnection") },
                    Action::CheckYandexConnection,
                ).enabled(preview && supported && bucket_ready
                    && ctx.yandex_keys == Some(true) && !ctx.yandex_probe_busy),
            ])),
    ];
    if !matches!(ctx.yandex_probe, ProbeView::NotChecked) {
        rows.push(info(t("settings.yandex.probeStatus"), None,
            Control::Status { text: probe_status, tone: probe_tone }));
    }
    if let Some(notice) = &ctx.yandex_notice {
        rows.push(info(t("settings.yandex.keyActionResult"), None,
            Control::Status { text: notice.clone(), tone: Tone::Secondary }));
    }
    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
    rows.extend(super::pairing::rows(ctx, supported && bucket_ready));
    Block { title: t("settings.yandex.title"), rows }
}
