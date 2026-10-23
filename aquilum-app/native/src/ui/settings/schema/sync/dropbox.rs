use super::*;
use aquilum_core::settings::models::DropboxReadOnlyDraft;

/// A single opt-in test profile, never a connected or trusted Vault.
pub(super) fn dropbox_setup(config: &AppConfig, ctx: &Context) -> Block {
    let draft = &config.dropbox_readonly;
    let ready = draft.ready();
    let available = ready && !ctx.dropbox_preview_busy;
    let rows = vec![
        info("Dropbox 3.13 — read-only test".into(),
            Some("Separate Dropbox App Folder; only a pinned synthetic Vault. No upload, delete or local document changes.".into()),
            Control::Status {
                text: ctx.dropbox_preview_status.clone(),
                tone: if ctx.dropbox_preview_status.contains("REJECTED")
                    || ctx.dropbox_preview_status.contains("FAILED") { Tone::Danger }
                    else { Tone::Secondary },
            }),
        row("OAuth app key".into(), Some("Public App Folder application ID; never paste a client secret.".into()),
            Control::Text { value: draft.oauth_app_key.clone(), commit: true },
            |c, v| {
                let value = text(v);
                if DropboxReadOnlyDraft::valid_app_key(&value) {
                    c.dropbox_readonly.oauth_app_key = value;
                }
            }),
        row("Registered callback port".into(),
            Some("Exact http://127.0.0.1:PORT/dropbox/callback must be registered in Dropbox.".into()),
            Control::Number(Number::new(f64::from(draft.registered_port), 1.0, 65535.0, 1.0)),
            |c, v| {
                let number = number(v);
                if number.is_finite() && (1.0..=65535.0).contains(&number)
                    && number.fract() == 0.0 {
                    c.dropbox_readonly.registered_port = number as u16;
                }
            }),
        row("Dropbox account ID".into(),
            Some("Exact dbid:… from independently verified test account.".into()),
            Control::Text { value: draft.account_id.clone(), commit: true },
            |c, v| {
                let value = text(v);
                if DropboxReadOnlyDraft::valid_account(&value) {
                    c.dropbox_readonly.account_id = value;
                }
            }),
        row("Root namespace ID".into(), None,
            Control::Text { value: draft.root_namespace_id.clone(), commit: true },
            |c, v| {
                let value = text(v);
                if DropboxReadOnlyDraft::valid_namespace(&value) {
                    c.dropbox_readonly.root_namespace_id = value;
                }
            }),
        row("Synthetic Vault ID".into(), None,
            Control::Text { value: draft.vault_id.clone(), commit: true },
            |c, v| {
                let value = text(v);
                if DropboxReadOnlyDraft::valid_vault(&value) {
                    c.dropbox_readonly.vault_id = value;
                }
            }),
        row("Test profile".into(), None,
            Control::Text { value: draft.profile_id.clone(), commit: true },
            |c, v| {
                let value = text(v);
                if DropboxReadOnlyDraft::valid_profile(&value) {
                    c.dropbox_readonly.profile_id = value;
                }
            }),
        row("Independently pinned HEAD BLAKE3".into(),
            Some("64 lower-case hex digits obtained outside Dropbox. Never use a digest supplied by the remote service.".into()),
            Control::Text { value: draft.trusted_head_blake3.clone(), commit: true },
            |c, v| {
                let value = text(v);
                if DropboxReadOnlyDraft::valid_pin(&value) {
                    c.dropbox_readonly.trusted_head_blake3 = value;
                }
            }),
        info("Explicit test (no sync)".into(),
            Some(if ready {
                "Browser authorization and cold restart readback use the native connector. Data stays read-only.".into()
            } else {
                "Fill all independent test-account fields; the real account and HEAD must already exist.".into()
            }),
            Control::Buttons(vec![
                Button::new("Authorize and verify (read-only)".into(), Action::DropboxConnectReadOnly)
                    .enabled(available),
                Button::new("Resume stored connection (read-only)".into(), Action::DropboxResumeReadOnly)
                    .enabled(available),
            ])),
    ];
    Block { title: "Dropbox — experimental".into(), rows }
}
