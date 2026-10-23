use super::*;

#[test]
fn numbers_parse_clamp_step_and_format() {
    let n = Number::new(1.6, 1.2, 2.0, 0.05);
    assert_eq!(n.parse("1,75"), Some(1.75));
    assert_eq!(n.parse("9"), Some(2.0));
    assert_eq!(n.parse(" "), None);
    assert_eq!(n.parse("abc"), None);
    assert_eq!(n.step_from(1.6, 1.0), 1.65);
    assert_eq!(n.step_from(2.0, 1.0), 2.0);
    assert_eq!(format_number(1.65), "1.65");
    assert_eq!(format_number(100.0), "100");
    assert_eq!(format_number(1.5), "1.5");
}

#[test]
fn setters_write_config() {
    let mut config = AppConfig::default();
    let ctx = Context::new(1.0);
    let theme = ui(&config, &ctx).remove(0).rows.remove(1);
    let Some(Setter::Config(set)) = theme.setter else { panic!("тема пишется в настройки") };
    set(&mut config, &Value::Index(2));
    assert_eq!(config.ui.theme, "dark");
    assert!(blocks(Section::Analysis, &config, &ctx).len() == 2);
    config.analysis.enable_bm25f = false;
    assert!(blocks(Section::Analysis, &config, &ctx).len() == 1);
}

#[test]
fn templates_path_and_trash_size() {
    let workspace = Path::new("/kb");
    assert_eq!(templates_path(workspace, ""), workspace.join("Templates"));
    assert_eq!(templates_path(workspace, "Шаблоны/"), workspace.join("Шаблоны"));
    assert_eq!(templates_path(workspace, "/Шаблоны/"), PathBuf::from("/Шаблоны"));
    assert_eq!(templates_path(workspace, "D:\\T\\"), PathBuf::from("D:\\T"));
    i18n::set_language("ru");
    assert_eq!(format_size(512), "512 Б");
    assert_eq!(format_size(1_572_864), "1,5 МБ");
}

#[test]
fn font_lists_start_with_the_bundled_families_and_go_on_with_installed_ones() {
    let list = families(&EDITOR_FAMILIES);
    assert_eq!(list[..3], ["iA Writer Mono", "iA Writer Quattro", "Inter"]);
    assert_eq!(family(&EDITOR_FAMILIES, &Value::Index(1)), "iA Writer Quattro");
    if let Some(installed) = list.get(3) {
        assert_eq!(&family(&EDITOR_FAMILIES, &Value::Index(3)), installed);
    }
    assert_eq!(family(&EDITOR_FAMILIES, &Value::Index(10_000)), "iA Writer Mono");
}

#[test]
fn templates_and_attachments_share_one_section_and_search_tuning_lives_with_bm25f() {
    let config = AppConfig::default();
    let ctx = Context::new(1.0);
    let titles: Vec<String> = blocks(Section::Data, &config, &ctx).into_iter().map(|b| b.title).collect();
    assert_eq!(titles, [t("settings.templates.section"), t("settings.files.section")]);
    for old in ["templates", "files", "data"] {
        assert_eq!(Section::from_key(old), Some(Section::Data), "сохранённый раздел «{old}» открывает общий раздел");
    }
    assert_eq!(Section::from_key("search"), Some(Section::Analysis));
    assert_eq!(Section::ALL.len(), 12);
    assert_eq!(Section::from_key("server"), Some(Section::Server));
    assert_eq!(Section::from_key("sync"), Some(Section::Sync));

    let rows = |config: &AppConfig| blocks(Section::Analysis, config, &ctx).into_iter().flat_map(|b| b.rows).map(|r| r.label).collect::<Vec<_>>();
    let pool = t("settings.search.candidatePool");
    assert!(rows(&config).contains(&pool), "настройки подбора похожих заметок стоят среди параметров BM25F");
    let mut off = AppConfig::default();
    off.analysis.enable_bm25f = false;
    assert!(!rows(&off).contains(&pool), "без BM25F этих настроек нет");
}


#[test]
fn m1dd_native_recovery_settings_are_read_only_for_every_state() {
    use aquilum_core::sync::NativeRecoveryOverview;
    let config = AppConfig::default();
    i18n::set_language("ru");
    let mut context = Context::new(1.0);
    for state in [
        None,
        Some(NativeRecoveryOverview::JournalNotConfigured),
        Some(NativeRecoveryOverview::NoLocalPublicationClaims),
        Some(NativeRecoveryOverview::InspectionUnavailable),
        Some(NativeRecoveryOverview::RequiresReview {
            total_claims: 2,
            examples: vec![
                aquilum_core::sync::NativeRecoveryExample {
                    operation_id: "11111111-1111-4111-8111-111111111111".into(),
                    historical_path: "notes/после-сбоя.md".into(),
                },
                aquilum_core::sync::NativeRecoveryExample {
                    operation_id: "22222222-2222-4222-8222-222222222222".into(),
                    historical_path: "notes/конфликт.md".into(),
                },
            ],
        }),
    ] {
        context.sync_recovery = state.clone();
        let all = blocks(Section::Sync, &config, &context);
        assert_eq!(all.len(), 2);
        let rows = &all[1].rows;
        assert!(rows.len() >= 3);
        assert!(rows.iter().all(|row| row.setter.is_none()));
        assert!(rows.iter().all(|row| match &row.control {
            Control::Status { .. } => true,
            Control::Buttons(buttons) =>
                buttons.iter().all(|b| matches!(&b.action, Action::InspectRecovery(_))),
            _ => false,
        }), "Only read-only inspection buttons may appear before explicit selection");
        assert!(rows.iter().all(|row| row.setter.is_none()));
        if let Some(NativeRecoveryOverview::RequiresReview { total_claims, .. }) = state {
            assert_eq!(total_claims, 2);
            assert!(rows.iter().any(|row| row.label == "notes/после-сбоя.md"));
        }
    }
    i18n::set_language("en");
}


#[test]
fn m1de_native_review_only_offers_explicit_nondestructive_intents() {
    use aquilum_core::sync::{
        NativeRecoveryCasePreview, NativeRecoveryDecision, NativeRecoveryExample,
        NativeRecoveryOverview,
    };
    let mut ctx = Context::new(1.0);
    let id = "11111111-1111-4111-8111-111111111111".to_string();
    ctx.sync_recovery = Some(NativeRecoveryOverview::RequiresReview {
        total_claims: 1,
        examples: vec![NativeRecoveryExample {
            operation_id: id.clone(),
            historical_path: "notes/not-lost.md".into(),
        }],
    });
    ctx.sync_selected = Some(NativeRecoveryCasePreview {
        operation_id: id,
        historical_path: "notes/not-lost.md".into(),
        assessment: "DifferentFinalFile".into(),
        archive_fingerprint: "a".repeat(64),
        review_count: 0,
        latest_event_digest: "genesis".into(),
    });
    let rows = &blocks(Section::Sync, &AppConfig::default(), &ctx)[1].rows;
    assert!(rows.iter().all(|r| r.setter.is_none()));
    let mut inspect = 0;
    let mut hold = 0;
    let mut escalate = 0;
    for row in rows {
        match &row.control {
            Control::Status { .. } => {}
            Control::Buttons(buttons) => for button in buttons {
                match &button.action {
                    Action::InspectRecovery(_) => inspect += 1,
                    Action::RequestRecoveryReview(NativeRecoveryDecision::KeepBlocked) => hold += 1,
                    Action::RequestRecoveryReview(NativeRecoveryDecision::EscalateManualReview) =>
                        escalate += 1,
                    other => panic!("disallowed recovery action: {other:?}"),
                }
            },
            other => panic!("writable settings control in review: {other:?}"),
        }
    }
    assert_eq!((inspect, hold, escalate), (1, 1, 1));
    ctx.sync_selected.as_mut().unwrap().operation_id = "different".into();
    let rows = &blocks(Section::Sync, &AppConfig::default(), &ctx)[1].rows;
    assert!(rows.iter().all(|r| match &r.control {
        Control::Status { .. } => true,
        Control::Buttons(buttons) => buttons.iter().all(|b| matches!(&b.action, Action::InspectRecovery(_))),
        _ => false,
    }), "forged selection cannot expose even a confirmation action");
}

#[cfg(not(all(target_os = "windows", feature = "s3-pair-beta")))]
#[test]
fn yandex_wizard_is_bounded_and_never_activates_s3_or_persists_a_secret() {
    let ctx = Context::new(1.0);
    let mut config = AppConfig::default();
    i18n::set_language("ru");
    let sections = blocks(Section::Sync, &config, &ctx);
    assert_eq!(sections.len(), 2);
    let setup = &sections[0];
    assert_eq!(setup.title, t("settings.yandex.title"));
    let mut links = 0;
    let mut disabled_probe = 0;
    let mut store_keys = 0;
    let mut remove_keys = 0;
    let mut writable_fields = 0;
    for row in &setup.rows {
        if row.setter.is_some() {
            writable_fields += 1;
            assert!(matches!(&row.control, Control::Text { commit: true, .. }),
                "Only committed nonsecret setup fields may persist");
        }
        if let Control::Buttons(buttons) = &row.control {
            for button in buttons {
                match &button.action {
                    Action::CheckYandexConnection => {
                        disabled_probe += 1;
                        assert!(!button.enabled, "blank bucket must forbid network check");
                    }
                    Action::OpenUrl(url) => {
                        links += 1;
                        assert!(url.starts_with("https://yandex.cloud/")
                            || *url == "https://console.yandex.cloud/",
                            "Only pinned HTTPS Yandex documentation destinations are allowed");
                    }
                    Action::StoreYandexKeys => {
                        store_keys += 1;
                        assert!(!button.enabled, "No bucket: cannot open credential UI");
                    }
                    Action::RemoveYandexKeys => {
                        remove_keys += 1;
                        assert!(!button.enabled, "No verified local credential: cannot remove");
                    }
                    other => panic!("S3 setup must not grant remote write/sync capability: {other:?}"),
                }
            }
        }
    }
    assert_eq!(writable_fields, 2);
    assert_eq!(links, 5);
    assert_eq!(disabled_probe, 1);
    assert_eq!((store_keys, remove_keys), (1, 1));
    assert!(setup.rows.iter().any(|row| row.label == t("settings.yandex.credentialSafety")));
    assert!(setup.rows.iter().any(|row| row.label == t("settings.yandex.explanation")));

    // The only editable fields are public bucket/profile names. They never
    // grant remote File Gate authority or select an actual credential provider.
    let bucket = setup.rows.iter().find(|row| row.label == t("settings.yandex.bucket")).unwrap();
    let profile = setup.rows.iter().find(|row| row.label == t("settings.yandex.profile")).unwrap();
    let Some(Setter::Config(set_bucket)) = bucket.setter else { panic!("bucket setter missing") };
    let Some(Setter::Config(set_profile)) = profile.setter else { panic!("profile setter missing") };
    set_bucket(&mut config, &Value::Text("private-akula-bucket".into()));
    set_profile(&mut config, &Value::Text("akula_yc".into()));
    assert_eq!(config.s3_onboarding.bucket, "private-akula-bucket");
    assert_eq!(config.s3_onboarding.aws_profile_name, "akula_yc");
    set_bucket(&mut config, &Value::Text("https://evil.test/secret".into()));
    set_profile(&mut config, &Value::Text("keys=not-safe".into()));
    assert_eq!(config.s3_onboarding.bucket, "private-akula-bucket");
    assert_eq!(config.s3_onboarding.aws_profile_name, "akula_yc");
    // Only a Windows user with a chosen bucket can invoke the native
    // OS credential dialog; the button never starts S3 network traffic.
    let user_controls = blocks(Section::Sync, &config, &ctx);
    let buttons = user_controls[0].rows.iter().flat_map(|row| match &row.control {
        Control::Buttons(items) => items.as_slice(),
        _ => &[],
    }).collect::<Vec<_>>();
    let save = buttons.iter().find(|b| b.action == Action::StoreYandexKeys).unwrap();
    let delete = buttons.iter().find(|b| b.action == Action::RemoveYandexKeys).unwrap();
    assert_eq!(save.enabled, cfg!(target_os = "windows"));
    assert!(!delete.enabled);
    let mut ctx_with_keys = Context::new(1.0);
    ctx_with_keys.yandex_keys = Some(true);
    let present = blocks(Section::Sync, &config, &ctx_with_keys);
    let buttons = present[0].rows.iter().flat_map(|row| match &row.control {
        Control::Buttons(items) => items.as_slice(),
        _ => &[],
    }).collect::<Vec<_>>();
    assert_eq!(buttons.iter().find(|b| b.action == Action::RemoveYandexKeys)
        .unwrap().enabled, cfg!(target_os = "windows"));
    assert!(present[0].rows.iter().any(|r| r.label == t("settings.yandex.keyStatus")));
    let check = buttons.iter().find(|b| b.action == Action::CheckYandexConnection).unwrap();
    assert_eq!(check.enabled,
        cfg!(all(target_os = "windows", feature = "yandex-s3-connection-preview")));
    ctx_with_keys.yandex_probe_busy = true;
    let busy = blocks(Section::Sync, &config, &ctx_with_keys);
    let busy_check = busy[0].rows.iter().flat_map(|row| match &row.control {
        Control::Buttons(items) => items.as_slice(), _ => &[],
    }).find(|b| b.action == Action::CheckYandexConnection).unwrap();
    assert!(!busy_check.enabled, "a single-flight check cannot start twice");
    ctx_with_keys.yandex_probe_busy = false;
    ctx_with_keys.yandex_probe = crate::yandex_probe_session::ProbeView::ReadOnlyListVerified;
    let success = blocks(Section::Sync, &config, &ctx_with_keys);
    assert!(success[0].rows.iter().any(|r| r.label == t("settings.yandex.probeStatus")),
        "read-only status should appear after an explicit worker completion");
    assert!(success[0].rows.iter().any(|r| r.label == t("settings.yandex.explanation")),
        "successful listing must not remove the explicit inactive Sync status");

    i18n::set_language("en");
    let english = blocks(Section::Sync, &config, &ctx);
    assert_eq!(english[0].title, t("settings.yandex.title"));
    assert!(english[0].rows.iter().any(|row| row.label == t("settings.yandex.credentialSafety")));
}


#[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
#[test]
fn s3_compact_setup_requires_keys_and_explicit_configure_before_sync() {
    let config = AppConfig::default();
    let mut ctx = Context::new(1.0);
    let sections = blocks(Section::Sync, &config, &ctx);
    let setup = &sections[0];
    assert_eq!(setup.title, t("settings.yandex.pairEntry"));
    let buttons = setup.rows.iter().flat_map(|row| match &row.control {
        Control::Buttons(items) => items.as_slice(), _ => &[],
    }).collect::<Vec<_>>();
    assert!(buttons.iter().find(|b| b.action == Action::StoreYandexKeys)
        .expect("native system keys button").enabled,
        "public bucket is captured and validated by the modal before WinCred opens");
    assert!(!buttons.iter().find(|b| b.action == Action::ConfigureS3Pair)
        .expect("configure after verified credential").enabled);
    assert!(!buttons.iter().any(|b| b.action == Action::SyncS3PairOnce),
        "Sync must not exist for an unregistered cloud profile");

    ctx.yandex_keys = Some(true);
    let buttons = blocks(Section::Sync, &config, &ctx)[0].rows.iter()
        .flat_map(|row| match &row.control {
            Control::Buttons(items) => items.as_slice(), _ => &[],
        }).map(|b| (b.action.clone(), b.enabled)).collect::<Vec<_>>();
    assert!(!buttons.iter().any(|(a, enabled)|
        *a == Action::ConfigureS3Pair && *enabled));
}

mod server;
#[test]
fn system_about_section_links_to_project_telegram_channel() {
    let sections = blocks(Section::System, &AppConfig::default(), &Context::new(1.0));
    let about = sections.last().expect("About section should be present");
    assert_eq!(about.rows.len(), 2, "project channel and app version");
    let telegram = &about.rows[0];
    assert_eq!(telegram.label, t("settings.system.projectTelegram"));
    let hint = t("settings.system.projectTelegramHint");
    assert_eq!(telegram.description.as_deref(), Some(hint.as_str()));
    assert!(telegram.setter.is_none());
    let Control::Buttons(buttons) = &telegram.control else {
        panic!("project Telegram channel must be a clickable link");
    };
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0].label, "https://t.me/AkulaHubl");
    assert_eq!(buttons[0].action, Action::OpenUrl("https://t.me/AkulaHubl"));
}

/// S3-E2E-01: remote VaultId is visible only in a separate, opt-in
/// diagnostic surface. A real pending file must never become a Restore,
/// ConfirmedBaseline or blind retry button.
#[cfg(feature = "s3-e2e01-review-preview")]
#[test]
fn s3_e2e01_bound_receive_status_is_visible_but_never_an_apply_button() {
    use aquilum_core::sync::{NativeRecoveryExample, NativeRecoveryOverview};
    let config = AppConfig::default();
    let mut ctx = Context::new(1.0);
    ctx.sync_recovery = Some(NativeRecoveryOverview::NoLocalPublicationClaims);
    ctx.s3_bound_publications = Some(NativeRecoveryOverview::RequiresReview {
        total_claims: 1,
        examples: vec![NativeRecoveryExample {
            operation_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),
            historical_path: "notes/received-over-s3.md".into(),
        }],
    });
    let sections = blocks(Section::Sync, &config, &ctx);
    let panel = sections.iter()
        .find(|block| block.title == t("settings.sync.title"))
        .unwrap();
    let unconfirmed = panel.rows.iter()
        .find(|row| row.label == "notes/received-over-s3.md")
        .unwrap();
    assert!(unconfirmed.setter.is_none());
    assert!(matches!(
        &unconfirmed.control,
        Control::Status { tone: Tone::Danger, .. }
    ));
    assert!(panel.rows.iter().any(|row| {
        row.label == t("settings.sync.s3BoundCount")
            && row.description.as_ref() == Some(&t("settings.sync.s3BoundNoAck"))
    }));
    assert!(!panel.rows.iter().any(|row| {
        matches!(&row.control, Control::Buttons(_))
            && row.label == t("settings.sync.s3BoundTitle")
    }));

    ctx.s3_bound_publications = Some(NativeRecoveryOverview::InspectionUnavailable);
    let blocked = blocks(Section::Sync, &config, &ctx);
    let panel = blocked.iter().find(|b| b.title == t("settings.sync.title")).unwrap();
    assert!(panel.rows.iter().any(|row| {
        row.label == t("settings.sync.s3BoundTitle")
            && matches!(&row.control, Control::Status { tone: Tone::Danger, .. })
    }));
    assert!(!panel.rows.iter().any(|row| {
        row.label == "notes/received-over-s3.md"
    }));
}
