//! Non-shipping Windows QA action: separate physical deletion after two
//! deliberate user clicks. Never reached from installed or regular Inbox.
use super::*;

fn confirmed_candidate(view: &InboxView, selected: &str) -> Option<(Uuid, u64, [u8; 32])> {
    let id = Uuid::parse_str(selected).ok()?;
    if id.is_nil() {
        return None;
    }
    let row = view
        .rows
        .iter()
        .find(|row| row.id == selected && row.status_key == "transfer.inboxVerified")?;
    if row.bytes == 0 || row.bytes > 8 * 1024 * 1024 || row.blake3 == [0; 32] {
        return None;
    }
    Some((id, row.bytes, row.blake3))
}

impl InboxPreview {
    pub(super) fn poll_physical_removal_result(&mut self, changed: &mut bool) {
        let outcome = self
            .physical_removal_rx
            .as_ref()
            .map(|receiver| receiver.try_recv());
        let pending = self.physical_removal_pending;
        match outcome {
            Some(Ok(Ok(id))) => {
                let confirmed = pending == Some(id);
                if let Some(selected) = pending {
                    if let Some(row) = self
                        .view
                        .rows
                        .iter_mut()
                        .find(|row| row.id == selected.to_string())
                    {
                        row.status_key = if confirmed {
                            "transfer.inboxRemoved"
                        } else {
                            "transfer.inboxReview"
                        };
                    }
                }
                self.view.notice_key = Some(if confirmed {
                    "transfer.inboxPhysicalRemoved"
                } else {
                    "transfer.inboxPhysicalReview"
                });
            }
            Some(Ok(Err(_))) | Some(Err(TryRecvError::Disconnected)) => {
                // The OS could complete unlink before a failed SQLite finish.
                // Never render Verified or claim Removed on an error.
                if let Some(selected) = pending {
                    if let Some(row) = self
                        .view
                        .rows
                        .iter_mut()
                        .find(|row| row.id == selected.to_string())
                    {
                        row.status_key = "transfer.inboxReview";
                    }
                }
                self.view.notice_key = Some("transfer.inboxPhysicalReview");
            }
            _ => return,
        }
        self.view.removing = false;
        self.view.preview = None;
        self.view.physical_removal_confirmation = None;
        self.physical_removal_pending = None;
        self.physical_removal_rx = None;
        *changed = true;
    }
}

impl App {
    pub(in crate::app::transfer) fn ask_inbox_physical_removal(&mut self, receipt: &str) -> bool {
        let view = &mut self.transfer.inbox.view;
        if view.loading
            || view.exporting
            || view.importing
            || view.previewing
            || view.removing
            || view.removal_confirmation.is_some()
            || view.has_physical_confirmation()
            || confirmed_candidate(view, receipt).is_none()
        {
            return false;
        }
        view.physical_removal_confirmation = Some(receipt.to_owned());
        view.notice_key = None;
        true
    }

    pub(in crate::app::transfer) fn cancel_inbox_physical_removal(&mut self) -> bool {
        self.transfer
            .inbox
            .view
            .physical_removal_confirmation
            .take()
            .is_some()
    }

    pub(in crate::app::transfer) fn begin_inbox_physical_removal(&mut self, receipt: &str) -> bool {
        let view = &mut self.transfer.inbox.view;
        if view.loading
            || view.exporting
            || view.importing
            || view.previewing
            || view.removing
            || view.removal_confirmation.is_some()
            || view.physical_removal_confirmation.as_deref() != Some(receipt)
        {
            return false;
        }
        let Some((id, bytes, digest)) = confirmed_candidate(view, receipt) else {
            return false;
        };
        let (tx, rx) = mpsc::channel();
        self.transfer.inbox.physical_removal_rx = Some(rx);
        self.transfer.inbox.physical_removal_pending = Some(id);
        view.removing = true;
        view.physical_removal_confirmation = None;
        view.notice_key = None;
        let wake = Arc::clone(&self.workspace.wake);
        thread::spawn(move || {
            let result = user_inbox_root()
                .map_err(|error| format!("Inbox root: {error:?}"))
                .and_then(|root| {
                    aquilum_core::p2p_inbox_lab::qa_dispose_confirmed_inbox_file(
                        &root, id, bytes, digest,
                    )
                })
                .map(|()| id);
            let _ = tx.send(result);
            wake();
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::transfer::InboxRow;

    fn sample() -> (InboxView, String) {
        let id = Uuid::new_v4().hyphenated().to_string();
        let mut view = InboxView::default();
        view.rows.push(InboxRow {
            id: id.clone(),
            bytes: 4,
            sender: "sender".to_owned(),
            kind_key: "transfer.inboxKindUnknown",
            status_key: "transfer.inboxVerified",
            import_audit_key: None,
            blake3: *blake3::hash(b"test").as_bytes(),
        });
        (view, id)
    }

    #[test]
    fn physical_candidate_refuses_stale_or_modified_selection() {
        let (mut view, id) = sample();
        assert!(confirmed_candidate(&view, &id).is_some());
        assert!(confirmed_candidate(&view, &Uuid::new_v4().to_string()).is_none());
        view.rows[0].status_key = "transfer.inboxReview";
        assert!(confirmed_candidate(&view, &id).is_none());
        view.rows[0].status_key = "transfer.inboxVerified";
        view.rows[0].blake3 = [0; 32];
        assert!(confirmed_candidate(&view, &id).is_none());
        view.rows[0].blake3 = *blake3::hash(b"test").as_bytes();
        view.rows[0].bytes = 0;
        assert!(confirmed_candidate(&view, &id).is_none());
    }
}
