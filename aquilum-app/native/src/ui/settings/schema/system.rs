use super::*;

pub(super) const PROJECT_TELEGRAM_URL: &str = "https://t.me/AkulaHubl";

pub(super) fn update_row(status: &crate::updater::Status) -> Row {
    use crate::updater::Status;
    let description = match status {
        Status::Idle => t("settings.system.updatesIdle"),
        Status::Checking => t("settings.system.updatesChecking"),
        Status::Latest => t("settings.system.updatesLatest"),
        Status::Available(version) => t_with("settings.system.updatesAvailable", &[("version", version)]),
        Status::Downloading { version, .. } | Status::Ready(version) => t_with("settings.system.updatesInstalling", &[("version", version)]),
        Status::Installing => t("update.installing"),
        Status::Failed(message) => t_with("settings.system.updatesFailed", &[("message", message)]),
    };
    let button = match status {
        Status::Available(_) => Button::new(t("settings.system.installUpdate"), Action::InstallUpdate),
        Status::Checking | Status::Downloading { .. } | Status::Ready(_) | Status::Installing => Button::new(t("settings.system.checkUpdates"), Action::CheckUpdates).enabled(false),
        _ => Button::new(t("settings.system.checkUpdates"), Action::CheckUpdates),
    };
    info(t("settings.system.manualUpdate"), Some(description), Control::Buttons(vec![button]))
}

pub(super) fn system(config: &AppConfig, ctx: &Context) -> Vec<Block> {
    let mut updates = vec![row(
        t("settings.system.autoUpdate"),
        Some(t("settings.system.autoUpdateHint")),
        Control::Switch(config.updates.auto),
        |c, v| c.updates.auto = flag(v),
    )];
    if !config.updates.auto {
        updates.push(match &ctx.update {
            Some(status) => update_row(status),
            None => info(
                t("settings.system.manualUpdate"),
                Some(t(if cfg!(feature = "installed") { "settings.system.updatesByPackage" } else { "settings.system.updatesUnavailable" })),
                Control::Buttons(vec![Button::new(t("settings.system.checkUpdates"), Action::CheckUpdates).enabled(false)]),
            ),
        });
    }
    let mut blocks = vec![
        Block { title: t("settings.system.updates"), rows: updates },
        Block {
            title: t("settings.system.autostart"),
            rows: vec![match ctx.autostart {
                Some(on) => Row { label: t("settings.system.launch"), description: Some(t("settings.system.launchHint")), control: Control::Switch(on), setter: Some(Setter::Autostart), code: None },
                None => info(t("settings.system.launch"), Some(t("settings.system.launchUnavailable")), Control::Locked(false)),
            }],
        },
        Block {
            title: t("settings.system.about"),
            rows: vec![
                info(
                    t("settings.system.projectTelegram"),
                    Some(t("settings.system.projectTelegramHint")),
                    Control::Buttons(vec![Button::new(PROJECT_TELEGRAM_URL.to_owned(), Action::OpenUrl(PROJECT_TELEGRAM_URL))]),
                ),
                info(
                    t("settings.system.version"),
                    Some(t("settings.system.versionHint")),
                    Control::Status { text: env!("CARGO_PKG_VERSION").to_owned(), tone: Tone::Muted },
                ),
            ],
        },
    ];
    #[cfg(windows)]
    blocks.insert(2, optional_ytdlp_block(ctx));
    blocks
}

#[cfg(windows)]
fn optional_ytdlp_block(ctx: &Context) -> Block {
    use crate::video::optional_ytdlp::{Status, VERSION, BYTES};
    let (status, tone, can_install, can_remove) = match &ctx.optional_ytdlp {
        Status::Missing => (t("settings.system.ytdlpMissing"), Tone::Secondary, true, false),
        Status::Downloading(done) => (
            t_with("settings.system.ytdlpDownloading", &[
                ("downloaded", &format!("{}", done / (1024 * 1024))),
                ("total", &format!("{}", BYTES / (1024 * 1024))),
            ]),
            Tone::Secondary, false, false,
        ),
        Status::Installed => (
            t_with("settings.system.ytdlpInstalled", &[("version", VERSION)]),
            Tone::Secondary, false, true,
        ),
        Status::Invalid => (t("settings.system.ytdlpInvalid"), Tone::Danger, false, true),
        Status::Failed(_) => (t("settings.system.ytdlpFailed"), Tone::Danger, true, false),
    };
    let actions = vec![
        Button::new(t("settings.system.ytdlpDownload"), Action::InstallYtDlp).enabled(can_install),
        Button::new(t("settings.system.ytdlpRemove"), Action::RemoveYtDlp).enabled(can_remove),
        Button::new(t("settings.system.ytdlpRefresh"), Action::RefreshYtDlp),
    ];
    Block {
        title: t("settings.system.ytdlpTitle"),
        rows: vec![
            info(t("settings.system.ytdlpStatus"), Some(t("settings.system.ytdlpHint")),
                 Control::Status { text: status, tone }),
            info(t("settings.system.ytdlpActions"),
                 Some(t("settings.system.ytdlpVersionHint")),
                 Control::Buttons(actions)),
        ],
    }
}
