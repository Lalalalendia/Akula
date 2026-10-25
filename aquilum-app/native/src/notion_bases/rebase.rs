//! C5b4 QA dry-run: compare a selected imported source snapshot with another.
//! No approve/write action is exported from this module.
use super::{bases_edits, load, BasesSession, Payload, Stage};
use aquilum_core::Core;
use std::sync::Arc;

impl BasesSession {
    /// The host must show an explicit confirmation dialog before calling this.
    /// The core rechecks every source field and the pinned File Gate hash.
    pub fn apply_compatible_snapshot(&mut self, core: Arc<Core>) -> Result<(),()> {
        if self.view.stage!=Stage::Ready || self.view.busy
            || self.has_pending_draft() {return Err(());}
        let preview=self.view.rebase_preview.as_ref().ok_or(())?;
        let source=self.view.source.clone().ok_or(())?;
        let old=self.view.digest.clone().ok_or(())?;
        let base=self.view.base_id.clone().ok_or(())?;
        let target=self.view.target.clone().ok_or(())?;
        let expected=self.disk_hash.as_deref().ok_or(())?;
        if !preview.can_offer_approval() || preview.old_digest!=old
            || preview.source_root!=source || preview.expected_local_hash!=expected
            || !self.view.choices.iter().any(|c|
                c.base_id==base && c.digest==preview.new_digest) {
            return Err(());
        }
        let next=preview.new_digest.clone();
        let hash=expected.to_owned();
        self.start(Stage::Saving, move || {
            bases_edits::apply_rebase_approved(&core,&target,&source,&old,
                &next,&hash,true).map_err(|_| ())?;
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
