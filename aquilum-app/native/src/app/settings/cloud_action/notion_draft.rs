//! Capture the actual mounted editor contents before explicit Bases save.
//! Keystrokes never write to disk. Enter is not a commit action.
use super::*;

impl App {
    pub(super) fn capture_notion_draft(
        &mut self, root:&mut RenderRoot, id:WidgetId, action:&ErasedAction,
        cloud:&mut CloudPopup,
    )->bool {
        if let Some(TextAction::Changed(text)|TextAction::Entered(text))=
            action.downcast_ref::<TextAction>()
            && matches!(cloud.bindings.get(&id),Some(Binding::BasesDraft))
        {
            let _=self.notion_bases.update_draft(text);
            return true;
        }
        if action.downcast_ref::<Pressed>().is_some() &&
            matches!(cloud.bindings.get(&id),
                Some(Binding::Button(Action::NotionBasesSaveDraft)))
        {
            // Synchronize the actual widget on mouse/key activation too; don't
            // assume Changed was already delivered or silently use older text.
            let text=cloud.bindings.iter().find_map(|(field,binding)|{
                if !matches!(binding,Binding::BasesDraft){return None;}
                root.get_widget(*field)?
                    .downcast::<TextEditor>().map(|e|e.inner().text())
            });
            let accepted=text.as_ref().is_some_and(|value|
                self.notion_bases.update_draft(value).is_ok());
            if !accepted {
                if text.is_none(){self.notion_bases.reject_draft_input();}
                let config=self.workspace.core.settings.get_config();
                let ctx=self.settings_context(crate::ui::settings::schema::Section::Sync,None,0);
                cloud.rebuild(root,&config,&ctx);
                return true;
            }
        }
        false
    }
}
