//! VIDEO-TOOLS-01k: localized installed Video Tools status, never launch authority.
//! PATH presence, owner-unapproved keys and CI signatures cannot unlock Download.
use super::App;
use masonry::app::RenderRoot;
#[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
use super::Popup;
#[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
use crate::i18n::t;
#[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
use crate::ui::dialog::ConfirmDialog;
#[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
use crate::video::tools_package_probe::{probe_installed_video_tools, ToolsDiscovery};
#[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
use masonry::kurbo::Point;

#[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
fn notice_keys(status: ToolsDiscovery) -> (&'static str, &'static str) {
    match status {
        ToolsDiscovery::Missing => ("editor.videoToolsMissingTitle", "editor.videoToolsMissing"),
        ToolsDiscovery::Unsafe => ("editor.videoToolsUnsafeTitle", "editor.videoToolsUnsafe"),
        ToolsDiscovery::PublisherNotApproved => ("editor.videoToolsUnapprovedTitle", "editor.videoToolsUnapproved"),
        ToolsDiscovery::IncompatibleVersion => ("editor.videoToolsIncompatibleTitle", "editor.videoToolsIncompatible"),
        ToolsDiscovery::ReadOnlyAttested => ("editor.videoToolsAttestedTitle", "editor.videoToolsAttested"),
    }
}

impl App {
    pub(super) fn optional_video_tools_notice(&mut self, root: &mut RenderRoot) -> bool {
        #[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
        {
            // OS-known protected Program Files product, checked natively.
            // ReadOnlyAttested is diagnostic only; no process gets queued.
            let (title, body) = notice_keys(probe_installed_video_tools());
            let (widget, dialog) = ConfirmDialog::new(
                &t(title), &t(body),
                &t("editor.table.studyClose"),
                &t("editor.table.studyOk"),
            );
            root.add_layer(widget, Point::ORIGIN);
            root.focus_on(Some(dialog.layer));
            self.popup = Some(Popup::PdfNotice { dialog });
            return true;
        }
        #[cfg(not(all(windows, feature = "installed", not(feature = "video-ui-e2e"))))]
        let _ = root;
        false
    }
}

#[cfg(all(test, windows, feature = "installed", not(feature = "video-ui-e2e")))]
mod tests {
    use super::*;

    #[test]
    fn all_five_native_states_show_distinct_russian_and_english_status() {
        let states = [
            ToolsDiscovery::Missing, ToolsDiscovery::Unsafe,
            ToolsDiscovery::PublisherNotApproved,
            ToolsDiscovery::IncompatibleVersion, ToolsDiscovery::ReadOnlyAttested,
        ];
        let mut distinct = std::collections::HashSet::new();
        for state in states {
            let (title, body) = notice_keys(state);
            assert!(distinct.insert((title, body)));
            for raw in [
                include_str!("../../../assets/locales/ru.json"),
                include_str!("../../../assets/locales/en.json"),
            ] {
                let locale: serde_json::Value = serde_json::from_str(raw).unwrap();
                for key in [title, body] {
                    let leaf = key.strip_prefix("editor.").unwrap();
                    assert!(!locale["editor"][leaf].as_str().unwrap_or("").is_empty(),
                        "missing Video Tools translation: {key}");
                }
            }
        }
    }
}
