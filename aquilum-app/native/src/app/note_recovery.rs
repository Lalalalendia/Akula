//! Fail-closed DESK-15 recovery UI: inspect in the disk worker, never auto-apply.
use std::{collections::{HashSet, VecDeque}, path::PathBuf, sync::{Arc, mpsc::{self, Sender, Receiver}}};
use uuid::Uuid;
use masonry::{app::RenderRoot, core::{ErasedAction, WidgetId}, kurbo::Point};
use super::{App, Outcome, Popup};
use crate::{note::{Note, bridge::{RecoveryCandidate, inspect_for_recovery}}, ui::dialog::{ConfirmDialog, DialogAnswer, DialogIds}};

struct Observation { workspace: Uuid, path: PathBuf, result: Result<Option<RecoveryCandidate>, String> }
pub(super) struct RecoveryUi {
    tx: Sender<Observation>, rx: Receiver<Observation>, deferred: VecDeque<Observation>,
    dismissed: HashSet<(Uuid, PathBuf, u64)>,
}
impl Default for RecoveryUi {
    fn default() -> Self {
        let (tx,rx)=mpsc::channel();
        Self {tx,rx,deferred:VecDeque::new(),dismissed:HashSet::new()}
    }
}
pub(super) struct RecoveryPrompt {
    dialog: DialogIds, workspace: Uuid, path: PathBuf, candidate: RecoveryCandidate,
}
impl App {
    pub(super) fn arm_note_recovery(&mut self, note: &mut Note) {
        let (Some(vault),Some(session))=(self.tree.root(),self.session.as_ref()) else {return};
        let (vault,workspace)=(vault.to_path_buf(),session.workspace_id);
        let (data,path)=(self.workspace.data_dir.clone(),note.path.clone());
        if let Err(error)=note.arm_shadow(&data,&vault,workspace) {
            report!("recovery snapshot unavailable: {error}");
            note.save_error=Some(format!("Recovery backup needs review: {error}"));
            return;
        }
        let (tx,wake)=(self.recovery.tx.clone(),Arc::clone(&self.workspace.wake));
        crate::disk::run(move||{
            let result=inspect_for_recovery(&data,&vault,workspace,&path).map_err(|e|e.to_string());
            if tx.send(Observation{workspace,path,result}).is_ok(){wake();}
        });
    }

    pub(super) fn capture_note_recovery(&mut self) {
        if let Some(note)=&mut self.note && let Err(error)=note.snapshot_unsaved(&self.text) {
            report!("shadow staging rejected: {error}");
            note.save_error=Some(format!("Recovery backup failed: {error}"));
        }
    }

    pub(super) fn poll_note_recovery(&mut self, root: &mut RenderRoot) {
        while let Ok(message)=self.recovery.rx.try_recv(){self.recovery.deferred.push_back(message);}
        if let Some(error)=self.note.as_ref().and_then(Note::shadow_error)
            && self.note.as_ref().and_then(|n|n.save_error.as_deref())!=Some(error.as_str())
        {
            if let Some(note)=&mut self.note {note.save_error=Some(error);}
            self.refresh_save_error(root);
        }
        if self.popup.is_some(){return}
        while let Some(message)=self.recovery.deferred.pop_front() {
            if self.session.as_ref().map(|s|s.workspace_id)!=Some(message.workspace)
                || self.note.as_ref().is_none_or(|n|!n.is(&message.path)){continue;}
            if self.note.as_ref().and_then(Note::clean_cache_hash).is_none() {
                self.recovery.deferred.push_front(message);
                return;
            }
            let candidate=match message.result {
                Ok(Some(candidate)) if !candidate.already_saved=>candidate,
                Ok(_)=>continue,
                Err(error)=>{
                    if let Some(note)=&mut self.note {
                        note.save_error=Some(format!("Recovery needs review: {error}"));
                    }
                    self.refresh_save_error(root);
                    continue;
                }
            };
            if !self.recovery.dismissed.insert((message.workspace,message.path.clone(),candidate.revision)){continue;}
            let file=message.path.file_name().unwrap_or_default().to_string_lossy();
            let (title,description,confirm)=if candidate.source_changed {
                ("Recovery conflict",format!("'{file}' changed since an unsaved draft was saved. Akula will not overwrite the file. The backup remains available in AppData."),"Show backup")
            } else {
                ("Unsaved changes found",format!("Recover the draft for '{file}' from the previous session? The source will be reverified before File Gate is allowed to save it."),"Restore draft")
            };
            let (layer,dialog)=ConfirmDialog::new(title,&description,"Later",confirm);
            root.add_layer(layer,Point::ORIGIN);
            root.focus_on(Some(dialog.layer));
            self.popup=Some(Popup::Recovery(RecoveryPrompt{
                dialog,workspace:message.workspace,path:message.path,candidate
            }));
            break;
        }
    }

    fn restore_reviewed(&mut self, root: &mut RenderRoot, state: &RecoveryPrompt)->bool {
        if self.session.as_ref().map(|s|s.workspace_id)!=Some(state.workspace)
            || self.note.as_ref().is_none_or(|n|!n.is(&state.path)||n.clean_cache_hash().is_none()){return false;}
        let Some(vault)=self.tree.root() else {return false};
        let latest=inspect_for_recovery(&self.workspace.data_dir,vault,state.workspace,&state.path);
        let Ok(Some(candidate))=latest else {return false};
        if candidate.revision!=state.candidate.revision
            || candidate.baseline!=state.candidate.baseline
            || candidate.source_changed || candidate.already_saved {return false;}
        // Only a user-confirmed action can promote the draft to File Gate.
        if let Some(note)=&self.note {note.approve_recovery_capture();}
        self.commit_text(root,candidate.body);
        true
    }

    pub(super) fn on_note_recovery_action(&mut self, root: &mut RenderRoot,
        id: WidgetId, action: &ErasedAction, state: RecoveryPrompt)->Outcome {
        let Some(answer)=state.dialog.answer(id,action) else {
            self.popup=Some(Popup::Recovery(state));return Outcome::default();
        };
        crate::ui::dialog::close(root,state.dialog.layer);
        if answer==DialogAnswer::Confirm {
            if state.candidate.source_changed {
                crate::system::reveal(&state.candidate.backup_path);
            } else if !self.restore_reviewed(root,&state) {
                if let Some(note)=&mut self.note {
                    note.save_error=Some("Recovery source changed; backup retained for manual review".into());
                }
                self.refresh_save_error(root);
            }
        }
        Outcome::default()
    }
}
