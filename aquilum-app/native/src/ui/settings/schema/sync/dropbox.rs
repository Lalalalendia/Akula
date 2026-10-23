use super::*;
use aquilum_core::settings::models::DropboxReadOnlyDraft;

/// User-initiated Dropbox read-only connection; not a trusted or synchronized Vault.
pub(in crate::ui::settings::schema) fn dropbox_setup(config: &AppConfig, ctx: &Context) -> Block {
    let draft = &config.dropbox_readonly;
    let ready = draft.ready();
    let available = ready && !ctx.dropbox_preview_busy;
    let rows = vec![
        info("Dropbox — подключение (только чтение)".into(),
            Some("Подключение Dropbox с проверкой данных. Загрузка, удаление и синхронизация файлов пока недоступны.".into()),
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
            Some("Идентификатор Dropbox-аккаунта (dbid:…).".into()),
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
        row("Идентификатор хранилища".into(), None,
            Control::Text { value: draft.vault_id.clone(), commit: true },
            |c, v| {
                let value = text(v);
                if DropboxReadOnlyDraft::valid_vault(&value) {
                    c.dropbox_readonly.vault_id = value;
                }
            }),
        row("Профиль подключения".into(), None,
            Control::Text { value: draft.profile_id.clone(), commit: true },
            |c, v| {
                let value = text(v);
                if DropboxReadOnlyDraft::valid_profile(&value) {
                    c.dropbox_readonly.profile_id = value;
                }
            }),
        row("Контрольный BLAKE3 HEAD".into(),
            Some("64 шестнадцатеричных символа из независимого доверенного источника.".into()),
            Control::Text { value: draft.trusted_head_blake3.clone(), commit: true },
            |c, v| {
                let value = text(v);
                if DropboxReadOnlyDraft::valid_pin(&value) {
                    c.dropbox_readonly.trusted_head_blake3 = value;
                }
            }),
        info("Подключение без синхронизации".into(),
            Some(if ready {
                "Авторизация откроется в браузере. Проверка подключения не изменяет файлы.".into()
            } else {
                "Заполните параметры аккаунта и проверяемого хранилища.".into()
            }),
            Control::Buttons(vec![
                Button::new("Подключить Dropbox (чтение)".into(), Action::DropboxConnectReadOnly)
                    .enabled(available),
                Button::new("Проверить сохранённое подключение".into(), Action::DropboxResumeReadOnly)
                    .enabled(available),
            ])),
    ];
    Block { title: "Dropbox".into(), rows }
}
