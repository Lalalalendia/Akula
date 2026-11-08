use super::*;

impl App {
    pub(super) fn on_rename_failed_popup(&mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction, dialog: DialogIds, reveal: PathBuf) {
        let closed = id == dialog.layer && action.downcast_ref::<DialogAnswer>() == Some(&DialogAnswer::Closed);
        let answer = if closed { Some(DialogAnswer::Cancel) } else { dialog.answer(id, action) };
        match answer {
            Some(answer) => {
                if closed { root.remove_layer(dialog.layer); }
                else { crate::ui::dialog::close(root, dialog.layer); }
                if answer == DialogAnswer::Confirm {
                    crate::system::reveal(&reveal);
                }
            }
            None => self.popup = Some(Popup::RenameFailed { dialog, reveal }),
        }
    }

    pub(super) fn on_pdf_notice_popup(&mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction, dialog: DialogIds) {
        match dialog.answer(id, action) {
            Some(_) => crate::ui::dialog::close(root, dialog.layer),
            None => self.popup = Some(Popup::PdfNotice { dialog }),
        }
    }
}
