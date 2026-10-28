use masonry::app::RenderRoot;
use super::{App, VersionCommand};
use crate::history::{self, Version};
use crate::i18n::t;
use crate::ui::backlinks::{self, VersionView};
use crate::ui::widgets::Slot;
impl App {
    pub(in crate::app) fn open_version(&mut self, root: &mut RenderRoot, version: Version) {
        self.opened_versions.insert(self.tabs.active_id(), version);
        self.version_notice = None;
        self.show_version(root);
        self.render_history(root);
    }
    pub(in crate::app) fn show_version(&mut self, root: &mut RenderRoot) -> bool {
        let Some(version) = self.opened_versions.get(&self.tabs.active_id()).cloned() else { return false };
        let Some(path) = self.tabs.active().path.clone() else { return false };
        let texts = history::texts(&self.workspace.core, &path, &version.id);
        let (lines, notice, previous) = match &texts {
            Some(texts) => (history::diff(texts.previous.as_deref(), &texts.text), self.version_notice.clone(), texts.previous.is_some()),
            None => (Vec::new(), Some(t("history.missing")), false),
        };
        let can_restore = texts.is_some() && !version.is_current;
        let (view, ids): (_, VersionView) =
            backlinks::version_view(&history::heading(&version), previous, can_restore, &lines, notice.as_deref());
        let Some(chrome) = &self.chrome else { return false };
        chrome.version_page.edit(root, |mut slot| Slot::set_child(&mut slot, view));
        self.version_view = Some(ids);
        self.show_page(root, 2);
        true
    }

    /// Remove the hidden diff widget tree, not just the action button handles.
    /// IndexedStack keeps inactive page children alive until the Slot is replaced.
    pub(in crate::app) fn discard_version_view(&mut self, root: &mut RenderRoot) {
        if self.version_view.take().is_some() {
            if let Some(chrome) = &self.chrome {
                chrome.version_page.edit(root, |mut slot| Slot::set_child(
                    &mut slot,
                    masonry::core::NewWidget::new(masonry::widgets::SizedBox::empty()).erased(),
                ));
            }
        }
    }

    pub(in crate::app) fn close_version(&mut self, root: &mut RenderRoot) {
        self.opened_versions.remove(&self.tabs.active_id());
        self.discard_version_view(root);
        self.show_page(root, 0);
        self.render_history(root);
    }
    pub(super) fn run_version_command(&mut self, root: &mut RenderRoot, version: &Version, command: VersionCommand) {
        let Some(path) = self.tabs.active().path.clone() else { return };
        if let Some(note) = &mut self.note
            && !super::guard::flush_clean(note, &self.workspace.core, &self.text)
        {
            self.report_failed_history_flush(root, version);
            return;
        }
        let result = match command {
            VersionCommand::Restore => history::restore(&self.workspace.core, &path, version),
            VersionCommand::Revert => history::revert(&self.workspace.core, &path, version),
            VersionCommand::Rename => return,
        };
        match result {
            Ok(_) => {
                if let Some(note) = &mut self.note
                    && let Some(text) = note.reload_after_restore()
                {
                    self.set_text(root, text);
                }
                if self.opened_versions.contains_key(&self.tabs.active_id()) {
                    self.close_version(root);
                }
                self.load_history(root);
            }
            Err(error) => {
                report!("история: {error:?}");
                self.version_notice = Some(t("history.actionFailed"));
                self.show_version(root);
            }
        }
    }
}
