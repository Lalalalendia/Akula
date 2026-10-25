//! C5b4 QA dry-run: compare a selected imported source snapshot with another.
//! Applying a fully compatible snapshot requires explicit user confirmation.
use super::{bases_edits, load, BasesSession, BasesView, Payload, Stage};
use std::collections::BTreeSet;
use aquilum_core::Core;
use std::sync::Arc;


/// Pure UI guard; Core repeats this exact-set check before any File Gate CAS.
pub fn manual_choices_complete(view:&BasesView)->bool{
    let Some(report)=view.rebase_preview.as_ref() else{return false};
    if report.conflicted_columns!=0 || report.conflicted_cells==0
        || report.conflicted_cells>10 || report.cells.is_empty(){return false;}
    let conflicts=report.cells.iter()
        .filter(|cell|cell.disposition==bases_edits::RebaseDisposition::SourceValueChanged)
        .map(|cell|(cell.row_id.clone(),cell.property_id.clone()))
        .collect::<BTreeSet<_>>();
    let selected=view.keep_local.iter().cloned().collect::<BTreeSet<_>>();
    report.conflicted_cells==conflicts.len()
        && view.keep_local.len()==selected.len()
        && selected==conflicts
}

impl BasesSession {
    /// For QA scope the UI exposes explicit Keep Local only when every
    /// conflict is a source value edit with unchanged schema and identity.
    /// A conflicted cell is never implicitly selected.
    pub fn toggle_keep_local(&mut self,row:&str,property:&str)->bool{
        if self.view.stage!=Stage::Ready || self.view.busy
            || self.has_pending_draft() {return false;}
        let Some(report)=self.view.rebase_preview.as_ref() else{return false};
        if report.conflicted_columns>0
            || !report.cells.iter().any(|cell|
                cell.row_id==row && cell.property_id==property
                && cell.disposition==bases_edits::RebaseDisposition::SourceValueChanged) {
            return false;
        }
        let key=(row.to_owned(),property.to_owned());
        if self.view.keep_local.contains(&key) {
            self.view.keep_local.retain(|choice|choice!=&key);
        } else {
            self.view.keep_local.push(key);
        }
        true
    }

    pub fn apply_compatible_snapshot(&mut self,core:Arc<Core>)->Result<(),()> {
        self.apply_snapshot(core,false)
    }
    pub fn apply_reviewed_snapshot(&mut self,core:Arc<Core>)->Result<(),()> {
        self.apply_snapshot(core,true)
    }
    fn apply_snapshot(&mut self, core:Arc<Core>, reviewed:bool)->Result<(),()> {
        if self.view.stage!=Stage::Ready || self.view.busy
            || self.has_pending_draft() {return Err(());}
        let preview=self.view.rebase_preview.as_ref().ok_or(())?;
        if reviewed {
            if !manual_choices_complete(&self.view) {return Err(());}
        }else if !preview.can_offer_approval() || !self.view.keep_local.is_empty() {
            return Err(());
        }
        let source=self.view.source.clone().ok_or(())?;
        let old=self.view.digest.clone().ok_or(())?;
        let base=self.view.base_id.clone().ok_or(())?;
        let target=self.view.target.clone().ok_or(())?;
        let expected=self.disk_hash.as_deref().ok_or(())?;
        if preview.old_digest!=old || preview.source_root!=source
            || preview.expected_local_hash!=expected
            || !self.view.choices.iter().any(|c|
                c.base_id==base && c.digest==preview.new_digest) {return Err(());}
        let next=preview.new_digest.clone();
        let decisions=self.view.keep_local.clone();
        let hash=expected.to_owned();
        self.start(Stage::Saving,move ||{
            if reviewed {
                bases_edits::apply_rebase_resolved_approved(&core,&target,
                    &source,&old,&next,&hash,&decisions,true).map_err(|_| ())?;
            } else {
                bases_edits::apply_rebase_approved(&core,&target,
                    &source,&old,&next,&hash,true).map_err(|_| ())?;
            }
            match load(&target,&source,&next,&base)? {
                Payload::Loaded(effective,disk_hash)=>
                    Ok(Payload::Rebased(effective,disk_hash)),
                _=>Err(()),
            }
        })
    }
    pub fn preview_new_snapshot(&mut self, new_digest: &str) -> Result<(), ()> {
        if self.view.stage != Stage::Ready || self.view.busy
            || self.has_pending_draft() || self.effective.is_none()
            || self.disk_hash.is_none() {return Err(());}
        let old = self.view.digest.clone().ok_or(())?;
        let base = self.view.base_id.as_deref().ok_or(())?;
        if old == new_digest || !self.view.choices.iter().any(|choice|
            choice.base_id == base && choice.digest == new_digest) {
            return Err(());
        }
        let next = new_digest.to_owned();
        let root = self.view.source.clone().ok_or(())?;
        let vault = self.view.target.clone().ok_or(())?;
        self.view.rebase_preview = None;
        self.view.keep_local.clear();
        self.start(Stage::Reviewing, move || {
            bases_edits::preview_rebase(&vault, &root, &old, &next)
                .map(Payload::Rebase).map_err(|_| ())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notion_bases::{BaseChoice, BasesView};
    use std::{path::PathBuf, sync::Arc};

    #[test]
    fn approved_worker_receipt_switches_pinned_digest_only_on_success() {
        use super::super::Completion;
        use aquilum_core::notion_api_probe::bases_edits::EffectiveBase;
        let mut s=BasesSession::new(Arc::new(|| {}));
        let root="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let target=PathBuf::from("test-vault");
        let next="b".repeat(64);
        s.generation=9;s.view.busy=true;s.view.stage=Stage::Saving;
        s.view.source=Some(root.into());s.view.target=Some(target.clone());
        s.view.base_id=Some("base".into());s.view.digest=Some("a".repeat(64));
        s.disk_hash=Some("c".repeat(64));
        let rebased=EffectiveBase {
            base_id:"base".into(),notion_source_id:"notion".into(),
            source_root:root.into(),source_digest:next.clone(),
            local_revision:4,columns:Vec::new(),rows:Vec::new(),
            saved_source_views:0,native_views_supported:false,
        };
        s.tx.send(Completion{
            generation:9,source:root.into(),target,
            result:Ok(Payload::Rebased(rebased,Some("d".repeat(64)))),
        }).unwrap();
        assert!(s.poll());
        assert_eq!(s.view.stage,Stage::Ready);
        assert_eq!(s.view.digest.as_deref(),Some(next.as_str()));
        assert_eq!(s.disk_hash.as_deref(),Some("d".repeat(64).as_str()));
        assert_eq!(s.effective.as_ref().unwrap().local_revision,4);
        assert!(s.view.rebase_preview.is_none());
    }
    #[test]
    fn keep_local_requires_every_value_conflict_and_never_structural_conflicts() {
        use aquilum_core::notion_api_probe::bases_edits::{
            RebasePreview, RebaseCell, RebaseDisposition,
        };
        let mut session=BasesSession::new(Arc::new(||{}));
        session.view.stage=Stage::Ready;
        session.view.rebase_preview=Some(RebasePreview{
            source_root:"root".into(),old_digest:"a".repeat(64),
            new_digest:"b".repeat(64),expected_local_hash:"c".repeat(64),
            local_revision:2,ready_cells:0,conflicted_cells:1,
            conflicted_columns:0,
            cells:vec![RebaseCell{row_id:"row".into(),property_id:"prop".into(),
                disposition:RebaseDisposition::SourceValueChanged}],
            columns:Vec::new(),writes_performed:false,
        });
        assert!(!manual_choices_complete(&session.view));
        assert!(!session.toggle_keep_local("other","prop"));
        assert!(session.toggle_keep_local("row","prop"));
        assert!(manual_choices_complete(&session.view));
        assert!(session.toggle_keep_local("row","prop"));
        assert!(!manual_choices_complete(&session.view));
        if let Some(r)=session.view.rebase_preview.as_mut() {
            r.cells[0].disposition=RebaseDisposition::PropertyTypeChanged;
        }
        assert!(!session.toggle_keep_local("row","prop"));
        assert!(!manual_choices_complete(&session.view));
    }
    #[test]
    fn invalid_preview_preserves_the_current_verified_local_grid() {
        use super::super::Completion;
        use aquilum_core::notion_api_probe::bases_edits::EffectiveBase;
        let mut s=BasesSession::new(Arc::new(|| {}));
        s.generation=3;
        let root="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let target=PathBuf::from("test-vault");
        s.view.stage=Stage::Reviewing;
        s.view.busy=true;
        s.view.source=Some(root.into());
        s.view.target=Some(target.clone());
        s.view.digest=Some("a".repeat(64));
        s.disk_hash=Some("c".repeat(64));
        s.effective=Some(EffectiveBase {
            base_id:"base".into(),notion_source_id:"notion".into(),
            source_root:root.into(),source_digest:"a".repeat(64),
            local_revision:2,columns:Vec::new(),rows:Vec::new(),
            saved_source_views:0,native_views_supported:false,
        });
        s.tx.send(Completion {
            generation:3,source:root.into(),target,
            result:Err(()),
        }).unwrap();
        assert!(s.poll());
        assert_eq!(s.view.stage,Stage::Ready);
        assert!(s.effective.is_some(),"a failed read-only comparison cannot discard valid old data");
        assert!(s.view.rebase_preview.is_none());
        assert!(!s.view.busy);
    }
    #[test]
    fn preview_never_starts_without_a_pinned_local_overlay_and_known_target() {
        let mut session = BasesSession::new(Arc::new(|| {}));
        session.view = BasesView {
            stage: Stage::Ready,
            source: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()),
            target: Some(PathBuf::from("vault")),
            base_id: Some("base".into()),
            digest: Some("a".repeat(64)),
            choices: vec![BaseChoice {
                base_id: "base".into(), digest: "b".repeat(64), label: "Base".into(),
            }],
            ..Default::default()
        };
        assert!(session.preview_new_snapshot(&"b".repeat(64)).is_err());
        assert!(session.view.rebase_preview.is_none());
    }
}
