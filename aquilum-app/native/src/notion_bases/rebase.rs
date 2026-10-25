//! C5b4 QA dry-run: compare a selected imported source snapshot with another.
//! No approve/write action is exported from this module.
use super::{bases_edits, BasesSession, Payload, Stage};

impl BasesSession {
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
