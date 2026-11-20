use super::*;
use crate::video::optional_ytdlp::Status;
fn label(prefix: &str, suffix: &str) -> String { t(&format!("settings.system.{prefix}{suffix}")) }
fn present(status: &Status, prefix: &str, version: &str, bytes: u64) -> (String, Tone, bool, bool) {
    match status {
        Status::Missing => (label(prefix, "Missing"), Tone::Secondary, true, false),
        Status::Downloading(done) => (t_with(&format!("settings.system.{prefix}Downloading"), &[
            ("downloaded", &format!("{:.1}", *done as f64 / 1_048_576.0)),
            ("total", &format!("{:.1}", bytes as f64 / 1_048_576.0))]), Tone::Secondary, false, false),
        Status::Installed => (t_with(&format!("settings.system.{prefix}Installed"), &[("version", version)]), Tone::Secondary, false, true),
        Status::Invalid => (label(prefix, "Invalid"), Tone::Danger, false, true),
        Status::Failed(_) => (label(prefix, "Failed"), Tone::Danger, true, false),
    }
}
pub(super) fn optional_ytdlp_block(ctx: &Context) -> Block {
    use crate::video::optional_ytdlp::{BYTES, VERSION};
    use crate::video::optional_ytdlp::quickjs;
    let (yt, yt_tone, yt_install, yt_remove) = present(&ctx.optional_ytdlp, "ytdlp", VERSION, BYTES);
    let (js, js_tone, js_install, js_remove) = present(&ctx.optional_quickjs, "quickjs", quickjs::VERSION, quickjs::BYTES);
    Block { title: t("settings.system.ytdlpTitle"), rows: vec![
        info(t("settings.system.ytdlpStatus"), Some(t("settings.system.ytdlpHint")), Control::Status { text: yt, tone: yt_tone }),
        info(t("settings.system.ytdlpActions"), Some(t("settings.system.ytdlpVersionHint")), Control::Buttons(vec![
            Button::new(t("settings.system.ytdlpDownload"), Action::InstallYtDlp).enabled(yt_install),
            Button::new(t("settings.system.ytdlpRemove"), Action::RemoveYtDlp).enabled(yt_remove),
            Button::new(t("settings.system.ytdlpRefresh"), Action::RefreshYtDlp)])),
        info(label("quickjs", "Status"), Some(label("quickjs", "Hint")), Control::Status { text: js, tone: js_tone }),
        info(label("quickjs", "Actions"), Some(label("quickjs", "VersionHint")), Control::Buttons(vec![
            Button::new(label("quickjs", "Download"), Action::InstallQuickJs).enabled(js_install),
            Button::new(label("quickjs", "Remove"), Action::RemoveQuickJs).enabled(js_remove),
            Button::new(label("quickjs", "Refresh"), Action::RefreshQuickJs)])),
    ] }
}
