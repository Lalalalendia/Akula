//! Native app acceptance routes R03 (sources) and R05 (writing).
//! Bound to AKULA-SCENARIO-ACCEPTANCE-01: tests use real files, UI actions,
//! saved bytes, and reopening; external Zotero/PDF source lookup is out of scope.
use super::*;

#[test]
fn journey_r03_research_keeps_quote_page_and_own_conclusion_separate() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let source_text = "# Источник\n\nАвтор: Исследователь, с. 42.\n\n## Цитата\n\n> Калориметрия: контрольная фраза из статьи.";
    let source = h.write_note("Источник.md", source_text);
    let research = h.write_note("Исследование.md",
        "# Исследование\n\n[[Источник#Цитата|Первоисточник]]\n\n## Мой вывод\n\nГипотеза требует проверки.");
    h.app.start_index();
    assert!(wait_until(250, 20, || {
        let status = h.app.workspace.core.search.status(Some(&vault.to_string_lossy()));
        status.state == aquilum_core::search::models::SearchIndexState::Ready
            && !status.updating
            && status.indexed_documents >= 3
    }), "индекс исследования не готов");

    h.app.open_search(&mut h.root);
    let field = match &h.app.popup {
        Some(Popup::Search(state)) => state.field_id(),
        _ => panic!("пользовательский поиск не открылся"),
    };
    h.send(field, crate::ui::editor::TextAction::Changed("калориметрия".into()));
    assert!(wait_until(250, 20, || {
        h.app.on_core_events(&mut h.root);
        matches!(&h.app.popup, Some(Popup::Search(state)) if state.result_count() >= 1)
    }), "поисковая фраза из источника не найдена");
    let selected = match &h.app.popup {
        Some(Popup::Search(state)) => state.selected_path(),
        _ => unreachable!(),
    };
    assert_eq!(selected, source, "поиск выбрал другой документ");
    h.app.search_key(&mut h.root, &Key::Named(NamedKey::Enter), Modifiers::empty());
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(source.as_path()));
    assert_eq!(h.body_text(), source_text, "цитата или номер страницы потерялись");

    assert!(h.app.open(&mut h.root, research.clone()));
    assert!(h.app.open_target(
        &mut h.root,
        crate::links::Target::Wiki("Источник#Цитата|Первоисточник".into()),
        false,
    ), "ссылка на первоисточник не работает");
    h.deliver();
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(source.as_path()));
    let back = h.app.note_ids.as_ref().expect("источник открыт").back;
    h.act(back, crate::ui::widgets::Pressed);
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(research.as_path()));

    let updated = "# Исследование\n\n[[Источник#Цитата|Первоисточник]]\n\n## Мой вывод\n\nГипотеза требует повторного опыта.";
    h.replace_body(updated);
    h.save_now();
    assert_eq!(std::fs::read_to_string(&research).unwrap(), updated);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), source_text,
        "изменение собственного вывода переписало первоисточник");
    let mut h = h.restarted(1.0);
    assert!(h.app.open(&mut h.root, research));
    assert_eq!(h.body_text(), updated);
}

#[test]
fn journey_r05_writer_uses_template_keeps_checklist_and_exports_pdf() {
    let mut h = Harness::new();
    let folder = h.app.templates_dir().expect("папка шаблонов");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("Статья.md"),
        "# Статья\n\n## План\n\n## Источники\n\n- [ ] Проверить факты\n- [ ] Проверить ссылки\n")
        .unwrap();
    let draft = h.write_note("Черновик.md", "");
    let source_note = h.write_note("Материал.md", "# Материал\n\nПервичный материал");
    assert!(h.app.open(&mut h.root, draft.clone()));
    h.app.open_templates(&mut h.root);
    h.settle();
    h.type_text("статья");
    assert!(h.app.templates_key(
        &mut h.root, &Key::Named(NamedKey::Enter), Modifiers::empty()
    ).is_some(), "шаблон статьи не выбран");
    h.deliver();
    let inserted = h.body_text();
    assert!(inserted.contains("Проверить факты") && inserted.contains("## Источники"),
        "шаблон не вставился: {inserted:?}");

    let final_text = format!(
        "{inserted}\n[[Материал|Исходный материал]]\n\n## Черновик\n\nТекст на русском и пример **выделения**.\n"
    );
    h.replace_body(&final_text);
    h.save_now();
    assert_eq!(std::fs::read_to_string(&draft).unwrap(), final_text,
        "редактор не сохранил шаблон и правки");

    assert!(h.app.open_target(
        &mut h.root, crate::links::Target::Wiki("Материал".into()), false,
    ), "ссылка на исходный материал не открылась");
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(source_note.as_path()));
    assert!(h.app.open(&mut h.root, draft.clone()));
    assert_eq!(h.body_text(), final_text);
    let pdf = crate::app::note_page::export::render(
        "Черновик", crop::Rope::from(final_text.as_str()),
        crate::ui::editor::Smart::default(), None
    ).expect("экспорт текста статьи в PDF");
    assert!(pdf.starts_with(b"%PDF"), "результат экспорта не PDF");
    assert!(pdf.len() > 100, "PDF неожиданно пуст");

    let mut h = h.restarted(1.0);
    assert!(h.app.open(&mut h.root, draft));
    assert_eq!(h.body_text(), final_text, "после перезапуска пропали ссылки или чеклист");
}
