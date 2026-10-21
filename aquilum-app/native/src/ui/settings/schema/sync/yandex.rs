use super::*;
use aquilum_core::settings::models::YandexS3Onboarding;
use crate::yandex_probe_session::{ProbeFailure, ProbeView};

/// YC-S3-T01, UI stage only. A validated draft is NOT authorization, remote
/// account identity proof, a live S3 HEAD, or permission to start syncing.
/// Explicitly no field accepts a secret: native TextEditor fields are plaintext.
pub(super) fn yandex_setup(config: &AppConfig, ctx: &Context) -> Block {
    let draft = &config.s3_onboarding;
    let bucket_ready = !draft.bucket.is_empty() && YandexS3Onboarding::valid_bucket(&draft.bucket);
    let supported = cfg!(target_os = "windows");
    let (key_status, key_tone) = if !supported {
        (t("settings.yandex.keysUnsupported"), Tone::Muted)
    } else if !bucket_ready {
        (t("settings.yandex.keysNeedBucket"), Tone::Secondary)
    } else {
        match ctx.yandex_keys {
            Some(true) => (t("settings.yandex.keysStoredUnverified"), Tone::Secondary),
            Some(false) => (t("settings.yandex.keysMissing"), Tone::Secondary),
            None => (t("settings.yandex.keysUnavailable"), Tone::Danger),
        }
    };
    // Read-only proof is narrower than remote Vault trust. All statuses are
    // fixed localized categories: NEVER display S3 object names or SDK errors.
    let (probe_status, probe_tone) = match ctx.yandex_probe {
        ProbeView::NotChecked => (t("settings.yandex.keysStoredUnverified"), Tone::Secondary),
        ProbeView::Checking => (t("settings.yandex.probeChecking"), Tone::Secondary),
        ProbeView::ReadOnlyListVerified => (t("settings.yandex.probeVerifiedReadOnly"), Tone::Secondary),
        ProbeView::Failed(reason) => (t(match reason {
            ProbeFailure::MissingKeys => "settings.yandex.probeMissingKeys",
            ProbeFailure::InvalidStoredKeys => "settings.yandex.probeInvalidStoredKeys",
            ProbeFailure::CredentialStoreUnavailable => "settings.yandex.probeKeyStoreUnavailable",
            ProbeFailure::KeysChangedDuringCheck => "settings.yandex.probeKeysChanged",
            ProbeFailure::CredentialsRejected => "settings.yandex.probeRejected",
            ProbeFailure::AccessDenied => "settings.yandex.probeDenied",
            ProbeFailure::BucketNotFound => "settings.yandex.probeMissingBucket",
            ProbeFailure::ClockSkew => "settings.yandex.probeClock",
            ProbeFailure::WrongRegion => "settings.yandex.probeRegion",
            ProbeFailure::Timeout => "settings.yandex.probeTimeout",
            ProbeFailure::NetworkOrTls => "settings.yandex.probeNetwork",
            ProbeFailure::ServiceUnavailable => "settings.yandex.probeServer",
        }), Tone::Danger),
    };
    // A preview feature may only be enabled in an isolated Windows QA build.
    // The normal installed Akula binary has this permanently disabled.
    let preview = cfg!(all(target_os = "windows", feature = "yandex-s3-connection-preview"));
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
    Block { title: t("settings.yandex.title"), rows }
}

