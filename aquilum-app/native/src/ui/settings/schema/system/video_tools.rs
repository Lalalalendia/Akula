use super::*;

pub(super) fn optional_ytdlp_block(ctx: &Context) -> Block {
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
