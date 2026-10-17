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
    assert_eq!(Section::ALL.len(), 11);
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
        assert_eq!(all.len(), 1);
        let rows = &all[0].rows;
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
    let rows = &blocks(Section::Sync, &AppConfig::default(), &ctx)[0].rows;
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
    let rows = &blocks(Section::Sync, &AppConfig::default(), &ctx)[0].rows;
    assert!(rows.iter().all(|r| match &r.control {
        Control::Status { .. } => true,
        Control::Buttons(buttons) => buttons.iter().all(|b| matches!(&b.action, Action::InspectRecovery(_))),
        _ => false,
    }), "forged selection cannot expose even a confirmation action");
}
