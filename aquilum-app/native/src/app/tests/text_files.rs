use super::*;

#[test]
fn text_files_are_listed_opened_edited_and_saved() {
    let mut h = Harness::new();
    let path = h.write_note("заметка.txt", "первая строка");
    h.app.tree.paths_changed(&mut h.root, std::slice::from_ref(&path));
    assert!(h.app.tree.names().iter().any(|name| name == "заметка.txt"), "текстовый файл не показан в дереве");
    assert!(h.app.open(&mut h.root, path.clone()), "текстовый файл не открылся");
    assert_eq!(h.body_text(), "первая строка");
    h.replace_body("первая строка\nвторая");
    h.save_now();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "первая строка\nвторая", "правка не сохранилась в .txt");
}

#[test]
fn text_file_keeps_its_extension_when_the_title_is_edited() {
    use std::path::Path;
    let (to, _) = crate::file_ops::title_target(Path::new("/база/заметка.txt"), "Новое: имя").unwrap();
    assert_eq!(to, Path::new("/база/Новое имя.txt"));
    let (to, _) = crate::file_ops::title_target(Path::new("/база/заметка.md"), "Новое").unwrap();
    assert_eq!(to, Path::new("/база/Новое.md"));
}

#[test]
fn text_file_follows_outside_edits_and_has_properties_like_a_note() {
    let mut h = Harness::new();
    h.watch_vault();
    let path = h.write_note("заметка.txt", "---\nтег: один\n---\nтекст");
    h.app.tree.paths_changed(&mut h.root, std::slice::from_ref(&path));
    assert!(h.app.open(&mut h.root, path.clone()));
    assert!(h.app.metadata_state().is_some(), "у .txt нет блока свойств, как у заметки");
    std::fs::write(&path, "---\nтег: один\n---\nизменён снаружи").unwrap();
    assert!(
        wait_until(300, 20, || {
            h.app.on_core_events(&mut h.root);
            h.body_text().contains("изменён снаружи")
        }),
        "правка .txt снаружи не подтянулась в редактор"
    );
}

#[test]
fn opening_a_folder_with_existing_txt_files_shows_and_opens_them_from_the_tree() {
    // The ordinary folder-picker route: the TXT files are already on disk
    // before Workspace::open and FileTree::new run.
    let folder = crate::test_dir::tempdir();
    let top = folder.path().join("Новый текстовый документ.txt");
    let upper = folder.path().join("С ПРОБЕЛАМИ.TXT");
    let nested = folder.path().join("Тексты");
    std::fs::create_dir(&nested).unwrap();
    let deep = nested.join("Вложенный.txt");
    std::fs::write(&top, "первая строка").unwrap();
    std::fs::write(&upper, "второй файл").unwrap();
    std::fs::write(&deep, "вложенный файл").unwrap();

    let mut h = Harness::with(Some(folder.path()), 1.0);
    assert_eq!(h.app.tree.root(), Some(folder.path()));
    for name in ["Новый текстовый документ.txt", "С ПРОБЕЛАМИ.TXT", "Тексты"] {
        assert!(h.app.tree.names().iter().any(|row| row == name), "{name}: пропал при первичном открытии папки");
    }

    let click = |h: &mut Harness, name: &str| {
        let id = h.app.tree.row_id(name).expect("строка видна в дереве");
        let center = h.root.get_widget(id).unwrap().ctx().bounding_box().center();
        h.click(center.x, center.y);
    };
    click(&mut h, "Новый текстовый документ.txt");
    assert_eq!(h.app.note.as_ref().map(|note| note.path.as_path()), Some(top.as_path()));
    assert_eq!(h.body_text(), "первая строка");

    click(&mut h, "С ПРОБЕЛАМИ.TXT");
    assert_eq!(h.app.note.as_ref().map(|note| note.path.as_path()), Some(upper.as_path()));
    assert_eq!(h.body_text(), "второй файл");

    click(&mut h, "Тексты");
    assert!(h.app.tree.names().iter().any(|row| row == "Вложенный.txt"));
    click(&mut h, "Вложенный.txt");
    assert_eq!(h.app.note.as_ref().map(|note| note.path.as_path()), Some(deep.as_path()));
    assert_eq!(h.body_text(), "вложенный файл");
}
