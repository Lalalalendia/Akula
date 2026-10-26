//! Acceptance routes R02 and R07 for Akula desktop.
use super::*;

#[test]
fn r02_course_index_opens_lecture_and_preserves_edited_cyrillic_notes() {
    let mut h = Harness::new();
    let course = h.write_note("Курс.md", "# Курс

## Лекции

[[Лекция 01#Тема]]");
    let lecture = h.write_note("Лекция 01.md", "# Лекция 01

## Тема

Первый конспект");
    assert!(h.app.open(&mut h.root, course.clone()));
    assert!(h.app.open_target(
        &mut h.root,
        crate::links::Target::Wiki("Лекция 01#Тема".into()),
        false,
    ));
    h.deliver();
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(lecture.as_path()));
    let updated = "# Лекция 01

## Тема

Объяснение: α + β. Русский пример.";
    h.replace_body(updated);
    h.save_now();
    assert_eq!(std::fs::read_to_string(&lecture).unwrap(), updated);
    assert!(h.app.open(&mut h.root, course));
    assert!(h.app.open(&mut h.root, lecture.clone()));
    assert_eq!(h.body_text(), updated);
    let mut h = h.restarted(1.0);
    assert!(h.app.open(&mut h.root, lecture));
    assert_eq!(h.body_text(), updated);
}

#[test]
fn r07_engineering_runbook_links_to_incident_and_preserves_commands() {
    let mut h = Harness::new();
    let runbook = h.write_note("Сервис.md",
        "# Сервис

## Диагностика

`systemctl status app`

[[Ошибка запуска#Решение]]");
    let incident = h.write_note("Ошибка запуска.md",
        "# Ошибка запуска

## Симптом

Нет ответа.

## Решение

`systemctl restart app`");
    let original = std::fs::read_to_string(&runbook).unwrap();
    assert!(h.app.open(&mut h.root, runbook.clone()));
    assert!(h.app.open_target(
        &mut h.root,
        crate::links::Target::Wiki("Ошибка запуска#Решение".into()),
        false,
    ));
    h.deliver();
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(incident.as_path()));
    let updated = "# Ошибка запуска

## Симптом

Нет ответа.

## Решение

`systemctl restart app`

Проверить журнал после перезапуска.";
    h.replace_body(updated);
    h.save_now();
    assert_eq!(std::fs::read_to_string(&incident).unwrap(), updated);
    assert!(h.app.open(&mut h.root, runbook.clone()));
    assert_eq!(h.body_text(), original);
    assert!(h.app.open(&mut h.root, incident.clone()));
    assert_eq!(h.body_text(), updated);
    let mut h = h.restarted(1.0);
    assert!(h.app.open(&mut h.root, incident));
    assert_eq!(h.body_text(), updated);
    assert_eq!(std::fs::read_to_string(runbook).unwrap(), original);
}
