//! SYNC-04g-M1d-e: explicit native confirmation of *metadata only*.
//! Neither choice restores, deletes, moves, exports or confirms any file.
//! The app session is not OS-level operator authentication.
use super::*;
use aquilum_core::sync::{
    native_recovery_case_preview, native_record_recovery_decision,
    NativeRecoveryDecision,
};
use crate::i18n::{t, t_with};
use crate::ui::dialog::ConfirmDialog;

impl App {
    pub(super) fn inspect_sync_incident(
        &mut self,
        dialog: &mut SettingsDialog,
        operation_id: &str,
    ) -> Effect {
        dialog.sync_selected = None;
        let Some(root) = self.tree.root() else {
            dialog.sync_notice = Some(t("settings.sync.reviewUnavailable"));
            return Effect::Content;
        };
        match native_recovery_case_preview(
            &self.workspace.core, &self.workspace.data_dir, root, operation_id,
        ) {
            Ok(preview) => {
                dialog.sync_selected = Some(preview);
                dialog.sync_notice = None;
            }
            Err(_) => {
                dialog.sync_notice = Some(t("settings.sync.reviewUnavailable"));
            }
        }
        Effect::Content
    }

    pub(super) fn request_sync_review(
        &mut self,
        root: &mut RenderRoot,
        dialog: &mut SettingsDialog,
        decision: NativeRecoveryDecision,
    ) -> Effect {
        if dialog.section != Section::Sync {
            return Effect::None;
        }
        let Some(selected) = dialog.sync_selected.as_ref() else {
            return Effect::None;
        };
        let Some(vault) = self.tree.root() else {
            dialog.sync_selected = None;
            dialog.sync_notice = Some(t("settings.sync.reviewUnavailable"));
            return Effect::Content;
        };
        // Immediately before showing the modal verify that the selected
        // snapshot still exists and has the same immutable archive and event
        // revision. Commit will independently verify it *again*.
        let Ok(fresh) = native_recovery_case_preview(
            &self.workspace.core, &self.workspace.data_dir,
            vault, &selected.operation_id,
        ) else {
            dialog.sync_selected = None;
            dialog.sync_notice = Some(t("settings.sync.reviewUnavailable"));
            return Effect::Content;
        };
        if &fresh != selected {
            dialog.sync_selected = None;
            dialog.sync_notice = Some(t("settings.sync.reviewStale"));
            return Effect::Content;
        }
        let verb = match decision {
            NativeRecoveryDecision::KeepBlocked => t("settings.sync.keepBlocked"),
            NativeRecoveryDecision::EscalateManualReview => t("settings.sync.escalate"),
        };
        let title = t("settings.sync.confirmTitle");
        // The original long path remains selectable in the settings detail
        // view. Avoid unbounded path text in the fixed-width confirmation.
        let display_path: String = selected.historical_path.chars().take(72).collect();
        let description = t_with("settings.sync.confirmDescription", &[
            ("path", &display_path),
            ("sequence", &selected.review_count.to_string()),
            ("action", &verb),
        ]);
        let (layer, ids) = ConfirmDialog::new(
            &title, &description, &t("common.cancel"), &verb,
        );
        root.add_layer(layer, Point::ORIGIN);
        root.focus_on(Some(ids.layer));
        dialog.sync_confirm = Some((ids, decision));
        Effect::None
    }

    pub(super) fn confirm_sync_review(
        &mut self,
        dialog: &mut SettingsDialog,
        decision: NativeRecoveryDecision,
    ) -> Effect {
        let Some(selected) = dialog.sync_selected.take() else {
            dialog.sync_notice = Some(t("settings.sync.reviewUnavailable"));
            return Effect::Content;
        };
        let Some(vault) = self.tree.root() else {
            dialog.sync_notice = Some(t("settings.sync.reviewUnavailable"));
            return Effect::Content;
        };
        // Fixed local, non-identity note. No "operator authenticated" claim.
        let note = match decision {
            NativeRecoveryDecision::KeepBlocked =>
                "Native UI confirmation: keep incident blocked and preserve all evidence",
            NativeRecoveryDecision::EscalateManualReview =>
                "Native UI confirmation: escalate to manual investigation; preserve all evidence",
        };
        match native_record_recovery_decision(
            &self.workspace.core, &self.workspace.data_dir, vault,
            &selected, decision, note,
        ) {
            Ok(receipt) => {
                dialog.sync_notice = Some(t_with(
                    "settings.sync.reviewRecorded", &[
                        ("sequence", &receipt.sequence.to_string()),
                    ],
                ));
            }
            Err(_) => {
                // A stale or corrupt journal must be re-inspected, never
                // silently retried from a cached UI intent.
                dialog.sync_notice = Some(t("settings.sync.reviewStale"));
            }
        }
        Effect::Content
    }
}
