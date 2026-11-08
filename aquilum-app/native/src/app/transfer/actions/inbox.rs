//! Inbox commands are local UI actions, never remote permissions.
use super::*;

impl App {
    pub(super) fn on_inbox_action(
        &mut self,
        dialog: &transfer_ui::TransferDialog,
        id: WidgetId,
    ) -> bool {
        let hit = dialog.inbox_save_actions.iter().find(|(button, _)| *button == id);
        let Some((_, command)) = hit else {
            return false;
        };
        match command {
            transfer_ui::InboxAction::Preview(receipt) => self.begin_inbox_text_preview(receipt),
            transfer_ui::InboxAction::SaveCopy(receipt) => self.begin_inbox_save(receipt),
            #[cfg(windows)]
            transfer_ui::InboxAction::Import(receipt) => self.begin_inbox_import(receipt),
            transfer_ui::InboxAction::AskRemoval(receipt) => self.ask_inbox_removal(receipt),
            transfer_ui::InboxAction::ConfirmRemoval(receipt) => self.begin_inbox_removal(receipt),
            transfer_ui::InboxAction::CancelRemoval => self.cancel_inbox_removal(),
        }
    }
}
