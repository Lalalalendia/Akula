//! Local-only credential lifecycle UI for personal Yandex Disk.
//! Do not call presence "connected": the server has not been contacted.
use super::*;

pub(super) fn extend_rows(ctx: &Context, rows: &mut Vec<Row>) {
    let saved_account = ctx.yandex_disk_saved_account.as_ref();
    rows.push(info(
        "Сохранённый Яндекс ID".into(),
        Some("Публичная привязка аккаунта к этой базе знаний. Она не содержит токенов и не доказывает действующий доступ к Яндексу.".into()),
        Control::Status {
            text: saved_account.map_or_else(
                || "Нет сохранённой привязки / SYNC_DISABLED".into(),
                |id| format!("Яндекс ID {id} · доступ НЕ проверен / SYNC_DISABLED"),
            ),
            tone: Tone::Muted,
        },
    ));
    let available = saved_account.is_some()
        && ctx.yandex_disk_app_configured
        && ctx.yandex_disk_vault_ready
        && !ctx.yandex_disk_busy
        && ctx.yandex_disk_review_account.is_none();
    rows.push(info(
        "Доступ на этом устройстве".into(),
        Some("Проверка читает только системное хранилище Windows/Linux. Даже найденный токен не означает, что OAuth-доступ всё ещё разрешён. «Забыть» удаляет локальный ключ после контрольного чтения, но не отзывает доступ на сайте Яндекса.".into()),
        Control::Buttons(vec![
            Button::new("Проверить локальный токен".into(),
                Action::YandexDiskInspectLocal).enabled(available),
            Button::new("Забыть на этом устройстве".into(),
                Action::YandexDiskForgetLocal).enabled(available),
        ]),
    ));
    rows.push(info(
        "Отозвать доступ у Яндекса".into(),
        Some("Для полного отзыва нужно отдельно управлять разрешениями приложения в Яндекс ID. Это не выполняется локальным удалением токена.".into()),
        Control::Buttons(vec![
            Button::new("Инструкция Яндекса по отзыву доступа".into(),
                Action::OpenUrl("https://yandex.ru/dev/id/doc/ru/tokens/token-invalidate")),
        ]),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_saved_identity_cannot_start_a_network_session() {
        let mut ctx = Context::new(1.0);
        ctx.yandex_disk_saved_account = Some("1000034426".into());
        ctx.yandex_disk_app_configured = false;
        ctx.yandex_disk_vault_ready = true;
        let mut rows = Vec::new();
        extend_rows(&ctx, &mut rows);
        let stored = rows.iter().find(|r| r.label == "Сохранённый Яндекс ID").unwrap();
        let Control::Status { text, .. } = &stored.control else { panic!("no status"); };
        assert!(text.contains("НЕ проверен"));
        let buttons = rows.iter().find(|r| r.label == "Доступ на этом устройстве").unwrap();
        let Control::Buttons(buttons) = &buttons.control else { panic!("no controls"); };
        assert!(buttons.iter().all(|b| !b.enabled));
        assert!(rows.iter().all(|r| r.setter.is_none()));
    }

    #[test]
    fn exact_saved_account_unlocks_only_explicit_local_actions() {
        let mut ctx = Context::new(1.0);
        ctx.yandex_disk_saved_account = Some("1000034426".into());
        ctx.yandex_disk_app_configured = true;
        ctx.yandex_disk_vault_ready = true;
        let mut rows = Vec::new();
        extend_rows(&ctx, &mut rows);
        let row = rows.iter().find(|r| r.label == "Доступ на этом устройстве").unwrap();
        let Control::Buttons(buttons) = &row.control else { panic!("no buttons"); };
        assert_eq!(buttons.len(), 2);
        assert!(buttons.iter().all(|b| b.enabled));
        assert!(matches!(buttons[0].action, Action::YandexDiskInspectLocal));
        assert!(matches!(buttons[1].action, Action::YandexDiskForgetLocal));
        ctx.yandex_disk_busy = true;
        let mut disabled = Vec::new();
        extend_rows(&ctx, &mut disabled);
        let row = disabled.iter().find(|r| r.label == "Доступ на этом устройстве").unwrap();
        let Control::Buttons(buttons) = &row.control else { panic!("no buttons"); };
        assert!(buttons.iter().all(|b| !b.enabled));
    }
}
