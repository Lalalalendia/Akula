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
        info("Приложение Dropbox".into(),
            Some("Используется официальное приложение Akula с OAuth PKCE. App Key и порт не вводятся вручную.".into()),
            Control::Status {
                text: "Akula · вход через браузер".into(),
                tone: Tone::Muted,
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
                "Для проверки существующего Vault укажите его идентификаторы и независимый BLAKE3 HEAD. Подключение аккаунта и синхронизация — разные этапы.".into()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_akula_oauth_identity_is_not_editable_in_settings() {
        let block = dropbox_setup(&AppConfig::default(), &Context::new(1.0));
        assert_eq!(block.title, "Dropbox");
        assert!(block.rows.iter().any(|row|
            row.label == "Приложение Dropbox" && row.setter.is_none()));
        assert!(block.rows.iter().all(|row|
            row.label != "OAuth app key" && row.label != "Registered callback port"));
        assert!(block.rows.iter().any(|row|
            row.label == "Контрольный BLAKE3 HEAD"));
    }
}
