//! Explicit Notion onboarding and read-only refresh; no remote or Vault writes.
use super::*;

impl App {
    pub(super) fn handle_notion_action(&mut self, action: &Action) -> bool {
        match action {
            Action::NotionConnect => {
                if self.notion_sync.busy() { return true; }
                match crate::notion_credentials::prompt_and_store() {
                    Ok(crate::notion_credentials::SaveOutcome::Saved(root)) => {
                        if self.notion_sync.start(self.workspace.data_dir.clone(), root).is_err() {
                            self.notion_sync.credential_failure();
                        }
                    }
                    Ok(crate::notion_credentials::SaveOutcome::Cancelled) => {}
                    Err(_) => self.notion_sync.credential_failure(),
                }
                true
            }
            Action::NotionRefresh(root) => {
                if !self.notion_sync.busy() &&
                    self.notion_sync.start(self.workspace.data_dir.clone(), root.clone()).is_err()
                {
                    self.notion_sync.credential_failure();
                }
                true
            }
            Action::NotionForget(root) => {
                if self.notion_sync.busy() { return true; }
                let confirmed = rfd::MessageDialog::new()
                    .set_title("Akula — Notion")
                    .set_description("Удалить сохранённый ключ доступа к этому корню Notion? Локальные страницы останутся доступными.")
                    .set_level(rfd::MessageLevel::Warning)
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show() == rfd::MessageDialogResult::Yes;
                if confirmed {
                    if crate::notion_credentials::remove(root).is_ok() {
                        self.notion_sync.forgotten(root);
                    } else {
                        self.notion_sync.credential_failure();
                    }
                }
                true
            }
            Action::NotionOpenRoot(root) => {
                let catalog = crate::notion_offline::NotionOfflineView::read(&self.workspace.data_dir);
                if catalog.error.is_none() && catalog.roots.iter().any(|item| &item.id == root) {
                    self.notion_session.open(self.workspace.data_dir.clone(), root.clone());
                }
                true
            }
            Action::NotionOpenPage(page) => {
                self.notion_session.page(page);
                true
            }
            Action::NotionPageBatch(first) => {
                self.notion_session.page_batch(*first);
                true
            }
            Action::NotionTextChunk(first) => {
                self.notion_session.text_chunk(*first);
                true
            }
            _ => false,
        }
    }
}
