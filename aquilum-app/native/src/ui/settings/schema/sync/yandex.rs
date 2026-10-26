use super::*;
use aquilum_core::settings::models::YandexS3Onboarding;
use crate::yandex_probe_session::{ProbeFailure, ProbeView};
mod controls;
#[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
mod pairing;

/// S3 setup accepts public metadata only; secrets stay in the OS credential store.
pub(in crate::ui::settings::schema) fn yandex_setup(config: &AppConfig, ctx: &Context) -> Block {
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
    let view = controls::ViewState {
        supported,
        bucket_ready,
        preview,
        key_status,
        key_tone,
        probe_status,
        probe_tone,
    };
    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
    {
        return pairing::compact(config, ctx, view);
    }
    #[cfg(not(all(target_os = "windows", feature = "s3-pair-beta")))]
    {
        controls::build(config, ctx, view)
    }
}
