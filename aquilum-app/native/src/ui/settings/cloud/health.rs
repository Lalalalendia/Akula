use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Health {
    NotConnected,
    Saved,
    Checking,
    Verified,
    Review,
    Unavailable,
}

impl Health {
    pub(super) fn label(self) -> String {
        t(match self {
            Self::NotConnected => "settings.cloud.disconnected",
            Self::Saved => "settings.cloud.saved",
            Self::Checking => "settings.cloud.checking",
            Self::Verified => "settings.cloud.verified",
            Self::Review => "settings.cloud.review",
            Self::Unavailable => "settings.cloud.unavailable",
        })
    }

    pub(super) fn color(self) -> Color {
        match self {
            Self::Verified => Color::new([0.17, 0.74, 0.36, 1.0]),
            Self::Saved => Color::new([0.87, 0.60, 0.17, 1.0]),
            Self::Checking => Color::new([0.20, 0.52, 0.91, 1.0]),
            Self::Review => Color::new([0.84, 0.27, 0.23, 1.0]),
            Self::NotConnected | Self::Unavailable => Color::new([0.53, 0.56, 0.60, 1.0]),
        }
    }
}

pub(super) fn health(provider: CloudProvider, config: &AppConfig, ctx: &Context) -> Health {
    #[cfg(not(feature = "dropbox-readonly-preview"))]
    let _ = config;
    #[cfg(feature = "dropbox-readonly-preview")]
    if provider == CloudProvider::Dropbox {
        if ctx.dropbox_preview_busy { return Health::Checking; }
        let status = ctx.dropbox_preview_status.as_str();
        if ["FAILED", "REJECTED", "DENIED", "MISMATCH", "REVOKED",
            "INVALID", "UNAVAILABLE", "NO_SAVED", "PUBLISHER_NOT_APPROVED"]
            .iter().any(|marker| status.contains(marker)) { return Health::Review; }
        return if config.dropbox_readonly.account_id.is_empty() {
            Health::NotConnected
        } else { Health::Saved };
    }
    #[cfg(feature = "notion-offline-preview")]
    if provider == CloudProvider::Notion {
        if ctx.notion_offline.error.is_some() { return Health::Review; }
        if !ctx.notion_offline.roots.is_empty() { return Health::Saved; }
        return Health::NotConnected;
    }
    if provider != CloudProvider::Yandex {
        return Health::Unavailable;
    }
    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
    {
        use crate::s3_pair_session::PairStatus;
        if ctx.s3_pair.busy { return Health::Checking; }
        if matches!(ctx.s3_pair.status,
            PairStatus::Failed
            | PairStatus::Transferred { needs_review: 1.., .. }
            | PairStatus::Transferred { received_unconfirmed: 1.., .. }
        ) {
            return Health::Review;
        }
        if ctx.s3_pair.configured && ctx.s3_pair.verified {
            return Health::Verified;
        }
        if ctx.s3_pair.configured || ctx.yandex_keys == Some(true) {
            return Health::Saved;
        }
    }
    Health::NotConnected
}
