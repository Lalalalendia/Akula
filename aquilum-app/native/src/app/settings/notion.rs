//! Explicit Notion onboarding and read-only refresh; no remote or Vault writes.
use super::*;

impl App {
    pub(super) fn handle_notion_action(&mut self, action: &Action) -> bool {
        match action {
            Action::NotionBasesOpen(source) => {
                if let Some(target) = self.workspace.root.clone() {
                    let _ = self.notion_bases.open(target, source.clone());
                }
                true
            }
            Action::NotionBasesSelect(digest, base_id) => {
                let _ = self.notion_bases.select(digest.clone(), base_id.clone());
                true
            }
            Action::NotionBasesPage(rows, columns) => {
                self.notion_bases.page(*rows, *columns);
                true
            }
            Action::NotionBasesSelectCell(row,property) => {
                self.notion_bases.select_cell(row,property);
                true
            }
            Action::NotionBasesBeginEdit(row,property) => {
                let _ = self.notion_bases.begin_draft(row, property);
                true
            }
            Action::NotionBasesNewColumn(kind) => {
                let _ = self.notion_bases.begin_new_column(*kind);
                true
            }
            Action::NotionBasesRenameColumn(property) => {
                let _ = self.notion_bases.begin_rename_column(property);
                true
            }
            Action::NotionBasesSaveSchema => {
                if self.notion_bases.view().target.as_deref()
                    == self.workspace.root.as_deref()
                {
                    let _ = self.notion_bases.commit_schema(
                        std::sync::Arc::clone(&self.workspace.core));
                }
                true
            }
            Action::NotionBasesCancelSchema => {
                self.notion_bases.cancel_schema();
                true
            }
            Action::NotionBasesSaveDraft => {
                if self.notion_bases.view().target.as_deref() == self.workspace.root.as_deref() {
                    let _ = self.notion_bases.commit_draft(
                        std::sync::Arc::clone(&self.workspace.core));
                }
                true
            }
            Action::NotionBasesCancelDraft => {
                self.notion_bases.cancel_draft();
                true
            }
            Action::NotionBasesReload => {
                let _ = self.notion_bases.reload();
                true
            }
            Action::NotionBasesChange(row, property, revert) => {
                if self.notion_bases.view().target.as_deref() == self.workspace.root.as_deref() {
                    let _ = self.notion_bases.change(
                        std::sync::Arc::clone(&self.workspace.core),
                        row, property, *revert,
                    );
                }
                true
            }
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
                if self.notion_bases.has_pending_draft() { return true; }
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
            Action::NotionReviewImport(root) => {
                if self.notion_sync.busy() || self.notion_import.busy()
                    || self.notion_session.view().root.as_deref() != Some(root.as_str())
                    || self.notion_session.view().state != crate::notion_offline::LoadState::Ready
                { return true; }
                if let Some(target) = &self.workspace.root {
                    if self.notion_import.review(self.workspace.data_dir.clone(),
                        root.clone(), target.clone()).is_err() {
                        self.notion_import.refused();
                    }
                } else { self.notion_import.refused(); }
                true
            }
            Action::NotionPublishImport(root) => {
                if self.notion_sync.busy() || self.notion_import.busy() { return true; }
                let Some(target) = self.workspace.root.as_deref() else { return true; };
                let view = self.notion_import.view();
                if !view.ready_for(root, target) { return true; }
                let confirmed = rfd::MessageDialog::new()
                    .set_title("Akula — независимый импорт Notion")
                    .set_description(format!(
                        "Создать {} Markdown-заметок и локальные ссылки в выбранной базе?\n\nБаза: {}\nИсточник: {}\nСнимок BLAKE3: {}\n\nСуществующие файлы не будут заменены. Исходные объекты, типы свойств и отдельный каталог устойчивых ID/связей будут сохранены через File Gate. Типизированная модель ({} источников, {} записей, {} видов) будет восстановлена из независимого JSON. Неразрешённые связи/свойства требуют проверки; редактируемые базы и медиа пока не восстановлены. Notion не изменится.",
                        view.pages, target.display(), root,
                        view.digest.as_deref().unwrap_or("неизвестно"),
                        view.base_schemas,view.base_rows,view.base_views,
                    ))
                    .set_level(rfd::MessageLevel::Warning)
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show() == rfd::MessageDialogResult::Yes;
                if confirmed && self.notion_import.publish(
                    std::sync::Arc::clone(&self.workspace.core), self.workspace.data_dir.clone(),
                    root.clone(), target.to_path_buf()
                ).is_err() {
                    self.notion_import.refused();
                }
                true
            }
            _ => false,
        }
    }
}
