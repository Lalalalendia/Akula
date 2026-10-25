//! Capture mounted native editors before an explicit Bases CAS write.
//! Neither typing nor Enter authorizes a File Gate mutation.
use super::*;

impl App {
    pub(super) fn capture_notion_draft(
        &mut self, root:&mut RenderRoot, id:WidgetId, action:&ErasedAction,
        cloud:&mut CloudPopup,
    )->bool {
        if let Some(TextAction::Changed(text)|TextAction::Entered(text))=
            action.downcast_ref::<TextAction>()
        {
            match cloud.bindings.get(&id) {
                Some(Binding::BasesDraft)=>{
                    let _=self.notion_bases.update_draft(text);
                    return true;
                }
                Some(Binding::BasesSchemaDraft)=>{
                    let _=self.notion_bases.update_schema_name(text);
                    return true;
                }
                _=>{}
            }
        }
        if action.downcast_ref::<Pressed>().is_some() {
            let schema=matches!(cloud.bindings.get(&id),
                Some(Binding::Button(Action::NotionBasesSaveSchema)));
            let cell=matches!(cloud.bindings.get(&id),
                Some(Binding::Button(Action::NotionBasesSaveDraft)));
            if schema || cell {
                // Read the actual mounted widget immediately before Save:
                // Changed events can be delayed, or focus lost to the button.
                let text=cloud.bindings.iter().find_map(|(field,binding)|{
                    let correct=if schema {
                        matches!(binding,Binding::BasesSchemaDraft)
                    } else {matches!(binding,Binding::BasesDraft)};
                    if !correct {return None;}
                    root.get_widget(*field)?
                        .downcast::<TextEditor>().map(|e|e.inner().text())
                });
                let accepted=text.as_ref().is_some_and(|value|if schema {
                    self.notion_bases.update_schema_name(value).is_ok()
                }else{
                    self.notion_bases.update_draft(value).is_ok()
                });
                if !accepted {
                    if text.is_none(){self.notion_bases.reject_draft_input();}
                    let config=self.workspace.core.settings.get_config();
                    let ctx=self.settings_context(
                        crate::ui::settings::schema::Section::Sync,None,0);
                    cloud.rebuild(root,&config,&ctx);
                    return true; // refuse Save; never use stale buffered text
                }
            }
        }
        false
    }
    /// A dismissed Notion popup must not silently discard a cell or schema
    /// draft. A File Gate save in flight cannot be cancelled by closing it.
    pub(super) fn confirm_close_notion_bases(&mut self) -> bool {
        let view = self.notion_bases.view();
        if view.stage == crate::notion_bases::Stage::Saving
            || (view.busy && self.notion_bases.has_pending_draft()) {
            return false;
        }
        if !self.notion_bases.has_pending_draft() { return true; }
        let confirmed = rfd::MessageDialog::new()
            .set_title("Akula Bases — незавершённая правка")
            .set_description("Отменить несохранённый черновик и закрыть окно? Локальные изменения в исходном Notion не записывались.")
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show() == rfd::MessageDialogResult::Yes;
        if confirmed {
            self.notion_bases.cancel_draft();
            self.notion_bases.cancel_schema();
        }
        confirmed
    }

}
