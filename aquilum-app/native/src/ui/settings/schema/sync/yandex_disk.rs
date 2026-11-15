//! Non-installed personal Yandex Disk REST sign-in surface. This provider
//! is unrelated to Yandex Cloud Object Storage / S3 credentials.

use super::*;

pub(super) fn yandex_disk_setup(ctx: &Context) -> Block {
    let in_review = ctx.yandex_disk_review_account.is_some();
    let can_start = ctx.yandex_disk_app_configured
        && ctx.yandex_disk_vault_ready
        && !ctx.yandex_disk_busy;
    let status = &ctx.yandex_disk_status;
    let danger = ["REJECTED", "FAILED", "UNAVAILABLE", "INVALID",
        "CHANGED", "MISSING", "NOT_REGISTERED", "NOT_SELECTED", "EXPIRED"]
        .iter().any(|word| status.contains(word));
    let mut rows = vec![
        info(
            "Личный Яндекс Диск · OAuth".into(),
            Some("Это личный Яндекс Диск через REST API, не Yandex Cloud Object Storage (S3). Авторизация не включает синхронизацию, загрузку или удаление заметок.".into()),
            Control::Status {
                text: status.clone(),
                tone: if danger { Tone::Danger } else { Tone::Secondary },
            },
        ),
        info(
            "Приложение Akula для Яндекса".into(),
            Some("В тестовой сборке должен быть заранее зарегистрирован публичный OAuth client_id и точный callback на 127.0.0.1. Они не редактируются в настройках.".into()),
            Control::Status {
                text: if ctx.yandex_disk_app_configured {
                    "Приложение зарегистрировано в сборке".into()
                } else {
                    "Не настроено: требуется регистрация OAuth приложения".into()
                },
                tone: if ctx.yandex_disk_app_configured {
                    Tone::Secondary
                } else {
                    Tone::Danger
                },
            },
        ),
        info(
            "Войти для проверки доступа".into(),
            Some("Откроется системный браузер; Akula проверит Яндекс ID и корень app:/ только для чтения. До отдельного подтверждения токен не сохранится.".into()),
            Control::Buttons(vec![
                Button::new("Подключить Яндекс Диск (чтение)".into(),
                    Action::YandexDiskLoginReadOnly).enabled(can_start),
            ]),
        ),
    ];
    if let Some(account) = ctx.yandex_disk_review_account.as_ref() {
        rows.push(info(
            "Подтвердить аккаунт Яндекса".into(),
            Some(format!("Проверен Яндекс ID {account}. Только после нажатия Akula сохранит refresh-токен в системном хранилище именно для этой локальной базы знаний; чтение и синхронизация файлов останутся отключены.")),
            Control::Buttons(vec![
                Button::new(
                    format!("Сохранить доступ для аккаунта {account}"),
                    Action::YandexDiskConfirmAccount,
                ).enabled(ctx.yandex_disk_vault_ready && !ctx.yandex_disk_busy && in_review),
            ]),
        ));
    } else {
        rows.push(info(
            "Подтверждение аккаунта".into(),
            Some("Сначала пройдите вход в браузере и проверьте, какой Яндекс ID вернулся. Никакого автоматического сохранения после OAuth нет.".into()),
            Control::Status {
                text: "Ожидается проверка аккаунта / SYNC_DISABLED".into(),
                tone: Tone::Muted,
            },
        ));
    }
    rows.push(info(
        "Локальная база знаний".into(),
        Some("Для изоляции системных ключей используется существующий UUID открытой базы из локального UiState. Смена базы отменяет подтверждение.".into()),
        Control::Status {
            text: if ctx.yandex_disk_vault_ready {
                "Локальный UUID определён".into()
            } else {
                "База знаний не выбрана или UUID недоступен".into()
            },
            tone: Tone::Muted,
        },
    ));
    Block {
        title: "Яндекс Диск · личный (экспериментальная авторизация)".into(),
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_product_oauth_identity_disables_login_and_sync() {
        let mut ctx = Context::new(1.0);
        ctx.yandex_disk_app_configured = false;
        ctx.yandex_disk_vault_ready = true;
        let block = yandex_disk_setup(&ctx);
        let login = block.rows.iter().find(|row|
            row.label == "Войти для проверки доступа").unwrap();
        let Control::Buttons(buttons) = &login.control else { panic!("button missing"); };
        assert!(buttons.iter().any(|button|
            matches!(button.action, Action::YandexDiskLoginReadOnly) && !button.enabled));
        assert!(block.rows.iter().all(|row| row.setter.is_none()));
    }

    #[test]
    fn approved_account_requires_second_separate_click() {
        let mut ctx = Context::new(1.0);
        ctx.yandex_disk_app_configured = true;
        ctx.yandex_disk_vault_ready = true;
        ctx.yandex_disk_review_account = Some("1000034426".into());
        ctx.yandex_disk_status = "YANDEX_REVIEW_ACCOUNT_BEFORE_SAVING / SYNC_DISABLED".into();
        let block = yandex_disk_setup(&ctx);
        let review = block.rows.iter().find(|row|
            row.label == "Подтвердить аккаунт Яндекса").unwrap();
        let Control::Buttons(buttons) = &review.control else { panic!("review button missing"); };
        assert!(buttons.iter().any(|button|
            matches!(button.action, Action::YandexDiskConfirmAccount) && button.enabled
            && button.label.contains("1000034426")));
        let mut busy = ctx;
        busy.yandex_disk_busy = true;
        let block = yandex_disk_setup(&busy);
        let review = block.rows.iter().find(|row|
            row.label == "Подтвердить аккаунт Яндекса").unwrap();
        let Control::Buttons(buttons) = &review.control else { panic!("review button missing"); };
        assert!(buttons.iter().all(|button| !button.enabled));
    }
}
