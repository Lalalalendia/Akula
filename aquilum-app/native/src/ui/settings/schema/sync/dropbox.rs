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
        info("Аккаунт Dropbox".into(),
            Some("Вход в официальный аккаунт без настройки Vault. Akula запросит только доступ для чтения; никакой синхронизации.".into()),
            Control::Buttons(vec![
                Button::new("Войти в Dropbox".into(), Action::DropboxLoginAccount)
                    .enabled(!ctx.dropbox_preview_busy),
                Button::new("Проверить сохранённый вход".into(), Action::DropboxResumeAccount)
                    .enabled(!draft.account_id.is_empty() && !draft.root_namespace_id.is_empty()
                        && !ctx.dropbox_preview_busy),
            ])),
        info("Сохранённый аккаунт".into(),
            Some("После перезапуска нажмите «Проверить сохранённый вход»: Akula использует защищённый токен, без браузера и без доверия к Vault.".into()),
            Control::Status {
                text: if draft.account_id.is_empty() {
                    "Нет авторизованного аккаунта".into()
                } else {
                    format!("{} · подтверждение требуется", draft.account_id)
                },
                tone: Tone::Muted,
            }),
        info("Управление авторизацией".into(),
            Some("«Забыть» удаляет локальный токен, но не отзывает разрешение на стороне Dropbox. Для полного отзыва воспользуйтесь страницей подключённых приложений.".into()),
            Control::Buttons(vec![
                Button::new("Забыть аккаунт на этом устройстве".into(), Action::DropboxForgetAccount)
                    .enabled(!draft.account_id.is_empty() && !ctx.dropbox_preview_busy),
                Button::new("Отозвать доступ на сайте Dropbox".into(),
                    Action::OpenUrl("https://www.dropbox.com/account/connected_apps")),
            ])),
        info("Дополнительно: проверка существующего Vault".into(),
            Some("Необязательный ручной режим для ранее созданного и независимо подтверждённого Vault. Не требуется для входа в аккаунт.".into()),
            Control::Status {
                text: "Проверка только для чтения · SYNC_DISABLED".into(),
                tone: Tone::Secondary,
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
        // Signing in must be available even without VaultId/profile/pin.
        let login = block.rows.iter().find(|row| row.label == "Аккаунт Dropbox")
            .expect("account entrypoint missing");
        let Control::Buttons(buttons) = &login.control else {
            panic!("account entrypoint is not a button");
        };
        assert!(buttons.iter().any(|button|
            matches!(&button.action, Action::DropboxLoginAccount) && button.enabled));
        assert!(buttons.iter().any(|button|
            matches!(&button.action, Action::DropboxResumeAccount) && !button.enabled));
        assert!(block.rows.iter().all(|row|
            row.label != "Dropbox account ID" && row.label != "Root namespace ID"));
        let local_logout = block.rows.iter()
            .find(|row| row.label == "Управление авторизацией")
            .expect("local credential removal is mandatory");
        let Control::Buttons(buttons) = &local_logout.control else {
            panic!("disconnect controls unavailable");
        };
        assert!(buttons.iter().any(|button|
            matches!(&button.action, Action::DropboxForgetAccount) && !button.enabled));
        let mut connected = AppConfig::default();
        connected.dropbox_readonly.account_id = "dbid:alice".into();
        connected.dropbox_readonly.root_namespace_id = "12345".into();
        let logged_in = dropbox_setup(&connected, &Context::new(1.0));
        let account = logged_in.rows.iter()
            .find(|row| row.label == "Аккаунт Dropbox").unwrap();
        let Control::Buttons(account_buttons) = &account.control else {
            panic!("no account buttons");
        };
        assert!(account_buttons.iter().any(|button|
            matches!(&button.action, Action::DropboxResumeAccount) && button.enabled));
        let logout = logged_in.rows.iter()
            .find(|row| row.label == "Управление авторизацией").unwrap();
        let Control::Buttons(buttons) = &logout.control else { panic!("missing logout"); };
        assert!(buttons.iter().any(|button|
            matches!(&button.action, Action::DropboxForgetAccount) && button.enabled));
    }
}
