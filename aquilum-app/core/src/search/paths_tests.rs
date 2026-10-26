use super::{
    canonical_workspace, is_hidden, is_markdown, relative_slash_path, same_path,
    same_workspace, strip_markdown_extension, strip_root,
};
use std::path::Path;

#[test]
fn accepts_markdown_case_insensitively() {
    assert!(is_markdown(Path::new("Note.MD")));
    assert!(!is_markdown(Path::new("Note.txt")));
}

#[cfg(windows)]
#[test]
fn paths_differing_only_by_cyrillic_case_are_the_same() {
    assert!(same_path(Path::new("C:/База/Идея.md"), Path::new(r"c:\БАЗА\идея.md")));
    assert!(!same_path(Path::new("C:/База/Идея.md"), Path::new("C:/База/Идеи.md")));
    assert_eq!(
        strip_root(Path::new("C:/БАЗА"), Path::new("c:/база/Проекты/Идея.md")),
        Some(Path::new("Проекты/Идея.md"))
    );
    assert_eq!(strip_root(Path::new("C:/База"), Path::new("C:/Другая/Идея.md")), None);
}

#[test]
fn relative_paths_use_forward_slashes_and_lose_only_the_markdown_extension() {
    let root = Path::new("vault");
    assert_eq!(relative_slash_path(root, &root.join("Проекты").join("Идея.md")), "Проекты/Идея.md");
    assert_eq!(strip_markdown_extension("Проекты/Идея.MD"), "Проекты/Идея");
    assert_eq!(strip_markdown_extension("рисунок.png"), "рисунок.png");
}

#[test]
fn only_service_folders_and_internal_staging_are_hidden() {
    let root = Path::new("C:/vault");
    for relative in [
        ".trash/note.md",
        ".aquilum/history/note.md",
        ".quantum/history/note.md",
        ".git/config",
        ".Git/objects/ab",
        ".obsidian/app.json",
        ".DS_Store",
        "notes/._metadata.md",
        ".AKULA-SYNC-STAGE-1111.md",
        ".aquilum-temp/partial.md",
        ".akula/data.md",
    ] {
        assert!(is_hidden(root, &root.join(relative)), "{relative}");
    }
    for relative in [
        "notes/note.md",
        ".заметки/Идея.md",
        "Проекты/.архив/Старая.md",
        ".hidden.md",
    ] {
        assert!(!is_hidden(root, &root.join(relative)), "{relative}");
    }
}

#[test]
fn the_markdown_walker_enters_user_dot_folders_not_internal_ones() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        ".заметки/Идея.md",
        ".git/SECRET.md",
        ".obsidian/Плагин.md",
        ".trash/Старая.md",
        ".quantum/Старая.md",
        ".akula-sync-stage-partial.md",
        "Проекты/.архив/Дело.md",
    ] {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"fixture").unwrap();
    }
    let mut found = super::markdown_files(dir.path())
        .map(|path| relative_slash_path(dir.path(), &path))
        .collect::<Vec<_>>();
    found.sort();
    assert_eq!(found, [".заметки/Идея.md", "Проекты/.архив/Дело.md"]);
}

#[test]
fn incomplete_sync_stages_are_not_indexed_as_markdown_documents() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.md"), b"good").unwrap();
    std::fs::write(
        dir.path().join(".akula-sync-stage-11111111-1111-4111-8111-111111111111.md"),
        b"untrusted partial publication",
    ).unwrap();
    let files: Vec<_> = super::markdown_files(dir.path()).collect();
    assert_eq!(files, vec![dir.path().join("note.md")]);
    let stage = dir.path().join(".AKULA-SYNC-STAGE-synthetic.md");
    assert!(super::is_hidden(dir.path(), &stage));
}

#[test]
fn note_history_versions_never_reach_search() {
    let root = Path::new("C:/vault");
    let version = "1790000000000_aaaaaaaa_agent.md";
    assert!(is_hidden(root, &Path::new("C:/vault/.aquilum/history/Идея.md").join(version)));
    assert!(is_hidden(root, &Path::new("C:/vault/.trash/.aquilum/history/Идея.md").join(version)));
}

#[test]
fn a_path_that_does_not_exist_yet_resolves_through_its_folder() {
    let directory = tempfile::tempdir().unwrap();
    let folder = super::canonical_path(directory.path());
    assert_eq!(super::canonical_path(&directory.path().join("Новая.md")), folder.join("Новая.md"));
}

#[test]
fn canonical_path_is_safe_for_frontend() {
    let directory = tempfile::tempdir().unwrap();
    let path = canonical_workspace(directory.path().to_str().unwrap()).unwrap();
    assert!(!path.to_string_lossy().starts_with(r"\\?\"));
    assert!(same_workspace(&path, path.to_str().unwrap()));
}

#[cfg(windows)]
#[test]
fn accepts_extended_workspace_paths() {
    let directory = tempfile::tempdir().unwrap();
    let root = canonical_workspace(directory.path().to_str().unwrap()).unwrap();
    let extended = format!(r"\\?\{}", root.display());
    assert!(same_workspace(&root, &extended));
}

#[test]
fn decomposed_and_composed_names_share_one_identity() {
    let composed = std::path::Path::new("Base/Мой план.md");
    let decomposed = std::path::Path::new("Base/Мои\u{306} план.md");
    assert_ne!(composed.to_str(), decomposed.to_str());
    assert_eq!(super::identity(composed), super::identity(decomposed));
}
