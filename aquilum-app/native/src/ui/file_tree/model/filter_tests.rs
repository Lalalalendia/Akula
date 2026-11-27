use super::*;
use aquilum_core::files::models::{FileItem, FileItemType};

fn tree() -> (tempfile::TempDir, TreeModel) {
    let dir = crate::test_dir::tempdir();
    let root = dir.path();
    std::fs::create_dir_all(root.join("Проекты/Вложенная")).unwrap();
    std::fs::create_dir_all(root.join("Архив/Старое")).unwrap();
    for name in ["README.md", "Проекты/Aquilum.md", "Проекты/Вложенная/глубоко.md", "Архив/Старое/старое.md", ".скрытый.md"] {
        std::fs::write(root.join(name), "").unwrap();
    }
    let model = TreeModel::new(Some(root.to_path_buf()), []);
    (dir, model)
}

fn names(model: &TreeModel) -> Vec<(String, usize)> {
    model.rows().iter().map(|row| (row.name.clone(), row.depth)).collect()
}

fn filtered(model: &mut TreeModel, root: &Path, query: &str) -> Vec<(String, usize)> {
    model.set_filter(query);
    model.set_index(vault::index(root));
    names(model)
}

#[test]
fn finds_closed_folder_items_and_preserves_parent_chain() {
    let (dir, mut model) = tree();
    model.set_filter("глуб");
    assert!(names(&model).is_empty(), "unexpanded directories are not scanned synchronously");
    model.set_index(vault::index(dir.path()));
    assert_eq!(names(&model), [("Проекты".into(), 0), ("Вложенная".into(), 1), ("глубоко".into(), 2)]);
    assert_eq!(model.rows()[2].marks, [(0, 8)]);
    assert_eq!(model.rows()[0].items, None);
}

#[test]
fn case_accent_and_multiword_matches() {
    let (dir, mut model) = tree();
    assert_eq!(filtered(&mut model, dir.path(), "READme"), [("README".into(), 0)]);
    assert_eq!(filtered(&mut model, dir.path(), "AQ ui"), [("Проекты".into(), 0), ("Aquilum".into(), 1)]);
    assert!(filtered(&mut model, dir.path(), "aq nonsense").is_empty());
    std::fs::write(dir.path().join("Café.md"), "").unwrap();
    assert_eq!(filtered(&mut model, dir.path(), "cafe"), [("Café".into(), 0)]);
}

#[test]
fn filtered_expansion_does_not_mutate_saved_expansion_state() {
    let (dir, mut model) = tree();
    filtered(&mut model, dir.path(), "глуб");
    model.press(0, Modifiers::default());
    assert!(model.expanded_paths().is_empty());
    model.set_filter("");
    assert!(!model.filtering() && !model.indexed());
    assert_eq!(names(&model).len(), 4, "original collapsed tree should return, including dotfile");
}

#[test]
fn sort_remains_consistent_while_filtering() {
    let (dir, mut model) = tree();
    filtered(&mut model, dir.path(), "а");
    model.set_sort(Sort::NameDesc);
    let order: Vec<_> = model.rows().iter().filter(|row| row.depth == 0).map(|row| row.name.clone()).collect();
    assert_eq!(order, ["Проекты", "Архив"]);
}

#[test]
fn filter_does_not_include_internal_files_or_symlinks() {
    let (dir, mut model) = tree();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join(".git/secret.md"), "").unwrap();
    assert!(filtered(&mut model, dir.path(), "secret").is_empty());
}

#[test]
fn synthetic_twenty_thousand_note_index_is_supported() {
    let (dir, mut model) = tree();
    let mut index = Lists::new();
    let mut dirs = Vec::new();
    for n in 0..400 {
        let folder = dir.path().join(format!("папка {n}"));
        let files: Vec<_> = (0..50).map(|i| {
            let name = format!("заметка {i} про кофе и чай {n}");
            FileItem { id: folder.join(&name).to_string_lossy().into_owned(), name, item_type: FileItemType::File }
        }).collect();
        index.insert(comparable(&folder), files);
        let name = format!("папка {n}");
        dirs.push(FileItem { id: dir.path().join(&name).to_string_lossy().into_owned(), name, item_type: FileItemType::Folder });
    }
    index.insert(comparable(dir.path()), dirs);
    model.set_filter("кофе чай");
    model.set_index(index);
    assert_eq!(model.rows().len(), 20_400);
}
