//! Acceptance R06, R08, R09: complete user journeys in the native desktop app.
//! Scope: local files, navigation, search, manual checklists, safe attachments.
//! Not in scope: multiplayer game boards, calendar services or backup recovery.
use super::*;

#[test]
fn journey_r06_game_session_links_npc_location_and_keeps_session_notes() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let world = h.write_note("Мир.md",
        "# Мир\n\n[[Арина|Персонаж]]\n\n[[Локация|Место встречи]]\n\n[[Сессия 07|Журнал]]");
    let npc_text = "# Арина\n\n## Цель\n\nНайти старую карту.\n\n[[Локация|Последнее место]]";
    let npc = h.write_note("Арина.md", npc_text);
    let place = h.write_note("Локация.md", "# Локация\n\nСеверные ворота.");
    let previous = h.write_note("Сессия 06.md", "# Сессия 06\n\nВстретили Арину.");
    let current = h.write_note("Сессия 07.md",
        "# Сессия 07\n\n[[Арина#Цель|Арина]] у [[Локация|ворот]].\n\n## Итоги\n\n- [ ] Узнать, где карта.");
    h.app.start_index();
    assert!(wait_until(250, 20, || {
        let status = h.app.workspace.core.search.status(Some(&vault.to_string_lossy()));
        status.state == aquilum_core::search::models::SearchIndexState::Ready
            && !status.updating && status.indexed_documents >= 6
    }), "заметки игровых сессий не попали в поиск");

    h.app.open_search(&mut h.root);
    let field = match &h.app.popup {
        Some(Popup::Search(state)) => state.field_id(),
        _ => panic!("окно поиска недоступно"),
    };
    h.send(field, crate::ui::editor::TextAction::Changed("Арина".into()));
    assert!(wait_until(250, 20, || {
        h.app.on_core_events(&mut h.root);
        matches!(&h.app.popup, Some(Popup::Search(state)) if state.result_count() >= 3)
    }), "поиск не видит персонажа в нескольких сессиях");
    assert!(h.app.search_key(&mut h.root, &Key::Named(NamedKey::Escape), Modifiers::empty()).is_some());

    assert!(h.app.open(&mut h.root, current.clone()));
    assert!(h.app.open_target(
        &mut h.root, crate::links::Target::Wiki("Арина#Цель|Арина".into()), false,
    ), "ссылка из журнала сессии не открыла персонажа");
    h.deliver();
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(npc.as_path()));
    assert_eq!(h.body_text(), npc_text);

    let back = h.app.note_ids.as_ref().expect("страница персонажа").back;
    h.act(back, crate::ui::widgets::Pressed);
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(current.as_path()));
    let updated = "# Сессия 07\n\n[[Арина#Цель|Арина]] у [[Локация|ворот]].\n\n## Итоги\n\n- [x] Узнали о старой карте.\n\nИтог записан.";
    h.replace_body(updated);
    h.save_now();
    assert_eq!(std::fs::read_to_string(&current).unwrap(), updated);
    assert_eq!(std::fs::read_to_string(&npc).unwrap(), npc_text,
        "итог сессии изменил описание персонажа");

    let mut h = h.restarted(1.0);
    assert!(h.app.open(&mut h.root, current));
    assert_eq!(h.body_text(), updated, "итог игровой сессии не пережил перезапуск");
    assert!(world.exists() && place.exists() && previous.exists(),
        "связанные страницы мира или прежней сессии потеряны");
}

#[test]
fn journey_r08_project_meeting_decision_search_and_release_checklist() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let project_text = "# Проект\n\n## Ссылки\n\n[[Встреча|Встреча команды]]\n\n[[Решение|Почему перенесли]]\n\n[[Выпуск|Проверки выпуска]]";
    let project = h.write_note("Проект.md", project_text);
    let meeting = h.write_note("Встреча.md",
        "# Встреча\n\nРассмотрели зависимость.\n\n[[Решение|Отдельная запись о решении]]");
    let decision_text = "# Решение\n\nОтложено из-за нехватки поставщиков.\n\nДата: 26.10.2026.";
    let decision = h.write_note("Решение.md", decision_text);
    let release = h.write_note("Выпуск.md",
        "# Выпуск\n\n- [ ] Проверить перенос данных\n- [ ] Проверить сборку");
    h.app.start_index();
    assert!(wait_until(250, 20, || {
        let status = h.app.workspace.core.search.status(Some(&vault.to_string_lossy()));
        status.state == aquilum_core::search::models::SearchIndexState::Ready
            && !status.updating && status.indexed_documents >= 5
    }), "индекс рабочих заметок не готов");

    h.app.open_search(&mut h.root);
    let field = match &h.app.popup {
        Some(Popup::Search(state)) => state.field_id(),
        _ => panic!("поиск недоступен"),
    };
    h.send(field, crate::ui::editor::TextAction::Changed("поставщиков".into()));
    assert!(wait_until(250, 20, || {
        h.app.on_core_events(&mut h.root);
        matches!(&h.app.popup, Some(Popup::Search(state)) if state.result_count() >= 1)
    }), "причина переноса не найдена");
    let selected = match &h.app.popup {
        Some(Popup::Search(state)) => state.selected_path(),
        _ => unreachable!(),
    };
    assert_eq!(selected, decision, "поиск вернул не решение");
    h.app.search_key(&mut h.root, &Key::Named(NamedKey::Enter), Modifiers::empty());
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(decision.as_path()));
    assert_eq!(h.body_text(), decision_text);

    assert!(h.app.open(&mut h.root, project.clone()));
    assert!(h.app.open_target(
        &mut h.root, crate::links::Target::Wiki("Выпуск|Проверки выпуска".into()), false,
    ));
    h.deliver();
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(release.as_path()));
    let checked = "# Выпуск\n\n- [x] Проверить перенос данных\n- [ ] Проверить сборку";
    h.replace_body(checked);
    h.save_now();

    assert_eq!(std::fs::read_to_string(&release).unwrap(), checked);
    assert_eq!(std::fs::read_to_string(&project).unwrap(), project_text);
    assert_eq!(std::fs::read_to_string(&decision).unwrap(), decision_text);
    let mut h = h.restarted(1.0);
    assert!(h.app.open(&mut h.root, release));
    assert_eq!(h.body_text(), checked, "отметка проверки пропала после перезапуска");
    assert!(meeting.exists(), "заметка встречи потеряна");
}

#[test]
fn journey_r09_trip_attaches_a_local_photo_and_reopens_day_notes() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let folder = vault.join("Поездка");
    std::fs::create_dir_all(&folder).unwrap();
    let plan = folder.join("План.md");
    let day = folder.join("День 1.md");
    let plan_text = "# Поездка\n\n[[День 1|Первый день]]\n\nБилет: https://example.org/ticket\n\n## Фотографии\n\n";
    let day_text = "# День 1\n\nМузей и прогулка.\n\nСохранить впечатления.";
    std::fs::write(&plan, plan_text).unwrap();
    std::fs::write(&day, day_text).unwrap();
    assert!(h.app.open(&mut h.root, plan.clone()));

    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(32, 20, image::Rgba([35, 90, 160, 255]))
        .write_to(&mut png, image::ImageFormat::Png).unwrap();
    let editor = h.app.note_ids.as_ref().expect("страница плана").body;
    h.root.focus_on(Some(editor.id()));
    assert!(h.app.paste_image(&mut h.root, png.get_ref()),
        "изображение не вставилось в заметку поездки");
    h.deliver();
    h.save_now();

    let content = std::fs::read_to_string(&plan).unwrap();
    assert!(content.contains("[[День 1|Первый день]]"));
    assert!(content.contains("https://example.org/ticket"));
    let embedded = content.lines().find(|line| line.starts_with("!["))
        .expect("после вставки отсутствует ссылка на изображение");
    let rel = embedded.split("](<").nth(1)
        .and_then(|path| path.strip_suffix(">)"))
        .or_else(|| embedded.split("](").nth(1)
            .and_then(|path| path.strip_suffix(')')))
        .expect("не удалось прочитать путь до изображения");
    let by_note = folder.join(rel);
    let by_vault = vault.join(rel);
    let photo = if by_note.is_file() { by_note } else { by_vault };
    assert!(photo.is_file(), "изображение сохранено не как локальный файл");
    assert!(std::fs::canonicalize(&photo).unwrap()
        .starts_with(std::fs::canonicalize(&vault).unwrap()),
        "вложение оказалось вне базы Akula");
    let bytes = std::fs::read(&photo).unwrap();
    assert!(image::load_from_memory(&bytes).is_ok(), "вложенная фотография повреждена");

    assert!(h.app.open_tree_file(&mut h.root, photo.clone(), false),
        "локальное вложение не открылось для просмотра");
    assert!(h.app.note.is_none(), "изображение не должно быть редактируемой заметкой");
    assert_eq!(std::fs::read(&photo).unwrap(), bytes, "просмотр изменил вложение");
    assert!(h.app.open(&mut h.root, day.clone()));
    assert_eq!(h.body_text(), day_text);
    assert!(h.app.open(&mut h.root, plan.clone()));
    assert_eq!(h.body_text(), content);

    let mut h = h.restarted(1.0);
    assert!(h.app.open(&mut h.root, plan));
    assert_eq!(h.body_text(), content, "ссылка на фотографию пропала после перезапуска");
    assert!(day.is_file() && photo.is_file());
}
