//! Strictly local Notion popup actions: no tokens, network, or Vault writes.
use super::*;

impl App {
    pub(super) fn handle_notion_action(&mut self, action: &Action) -> bool {
        match action {
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
