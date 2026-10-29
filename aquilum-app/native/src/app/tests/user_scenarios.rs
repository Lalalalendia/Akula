//! End-to-end acceptance journeys traced to AKULA-SCENARIO-ACCEPTANCE-01.
//! These exercise the actual native application harness, not standalone parsers.
use super::*;

#[test]
fn journey_r01_daily_note_is_saved_after_switching_tabs_and_restarting() {
    let mut h = Harness::new();
    let starter = h.vault.path().join("a.md");
    assert!(h.app.open(&mut h.root, starter.clone()));
    assert!(h.app.new_note(&mut h.root), "создание заметки из приложения не сработало");

    let journal = h.app.note.as_ref().expect("новая заметка открыта").path.clone();
    assert!(journal.exists(), "новая заметка не создана в пользовательской папке");
    let expected = "## Сегодня\n\n09:10 Записал мысль об исследовании.\n\n## Итог\n\nПроверить источники.";
    h.replace_body(expected);
    h.save_now();
    assert_eq!(std::fs::read_to_string(&journal).unwrap(), expected, "текст не записан на диск");

    assert!(h.app.open(&mut h.root, starter));
    assert!(h.app.open(&mut h.root, journal.clone()));
    assert_eq!(h.body_text(), expected, "после переключения вкладки потерян текст");

    let mut h = h.restarted(1.0);
    assert!(h.app.open(&mut h.root, journal.clone()), "после запуска заметка не открылась");
    assert_eq!(h.body_text(), expected, "после нового запуска изменился текст");
    assert_eq!(std::fs::read_to_string(journal).unwrap(), expected);
}

#[test]
fn journey_r04_wiki_index_opens_a_heading_and_returns_without_creating_files() {
    let mut h = Harness::new();
    let index = h.write_note("Указатель.md", "# Проект\n\n[[Тема#Вывод|Ключевой вывод]]");
    let target_text = "# Тема\n\nФакты из источника.\n\n## Вывод\n\nСобственное объяснение.";
    let target = h.write_note("Тема.md", target_text);
    assert!(h.app.open(&mut h.root, index.clone()));
    assert!(h.app.open_target(
        &mut h.root,
        crate::links::Target::Wiki("Тема#Вывод|Ключевой вывод".into()),
        false,
    ), "переход из указателя не сработал");
    h.deliver();

    assert_eq!(h.app.tabs.active().path.as_deref(), Some(target.as_path()));
    assert_eq!(caret(&h), "# Тема\n\nФакты из источника.\n\n".len(),
        "курсор не попал к нужному заголовку");
    let back = h.app.note_ids.as_ref().expect("страница темы открыта").back;
    h.act(back, crate::ui::widgets::Pressed);
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(index.as_path()),
        "кнопка назад не вернула в исходную заметку");
    assert_eq!(h.body_text(), "# Проект\n\n[[Тема#Вывод|Ключевой вывод]]");
    assert_eq!(std::fs::read_to_string(target).unwrap(), target_text);
    assert_eq!(
        std::fs::read_dir(h.vault.path()).unwrap()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "md"))
            .count(),
        3,
        "переход создал лишний файл"
    );
}
