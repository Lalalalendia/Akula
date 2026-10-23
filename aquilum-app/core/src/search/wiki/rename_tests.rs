use super::plan_for_rename;
use crate::search::wiki::{index_document, open_schema};
use rusqlite::Connection;
use std::fs;

#[test]
fn preserves_aliases_when_a_target_is_renamed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let old_path = root.join("Old.md");
    let new_path = root.join("New.md");
    let source_path = root.join("Source.md");
    fs::write(&old_path, "target").unwrap();
    fs::write(&source_path, "[[Old]] and [[Old|visible]]").unwrap();
    let mut connection = Connection::open_in_memory().unwrap();
    open_schema(&connection).unwrap();
    let transaction = connection.transaction().unwrap();
    index_document(&transaction, root, &old_path, "target").unwrap();
    index_document(
        &transaction,
        root,
        &source_path,
        "[[Old]] and [[Old|visible]]",
    )
    .unwrap();
    transaction.commit().unwrap();

    let plan = plan_for_rename(
        &connection,
        root,
        &old_path,
        &new_path,
        vec![source_path],
        None,
    )
    .unwrap();

    assert_eq!(plan[0].content, "[[New]] and [[New|visible]]");
}

#[test]
fn rename_keeps_anchors_labels_and_ignores_attachments() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let old_path = root.join("Old.md");
    let new_path = root.join("New.md");
    let source_path = root.join("Source.md");
    let text = "[[Old#Раздел]] [[Old#^abc|метка]] [[Old | пробелы ]] ![[Old.png|200]] [[#Раздел]]";
    fs::write(&old_path, "target").unwrap();
    fs::write(&source_path, text).unwrap();
    let mut connection = Connection::open_in_memory().unwrap();
    open_schema(&connection).unwrap();
    let transaction = connection.transaction().unwrap();
    index_document(&transaction, root, &old_path, "target").unwrap();
    index_document(&transaction, root, &source_path, text).unwrap();
    transaction.commit().unwrap();

    let plan = plan_for_rename(&connection, root, &old_path, &new_path, vec![source_path], None)
        .unwrap();

    assert_eq!(
        plan[0].content,
        "[[New#Раздел]] [[New#^abc|метка]] [[New | пробелы ]] ![[Old.png|200]] [[#Раздел]]"
    );
}

#[test]
fn rewrites_links_in_a_crlf_note_and_keeps_the_hash_of_its_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let old_path = root.join("Old.md");
    let new_path = root.join("New.md");
    let source_path = root.join("Source.md");
    let raw = "# Title\r\n\r\nsee [[Old]]\r\n";
    fs::write(&old_path, "target").unwrap();
    fs::write(&source_path, raw).unwrap();
    let mut connection = Connection::open_in_memory().unwrap();
    open_schema(&connection).unwrap();
    let transaction = connection.transaction().unwrap();
    index_document(&transaction, root, &old_path, "target").unwrap();
    index_document(&transaction, root, &source_path, "# Title\n\nsee [[Old]]\n").unwrap();
    transaction.commit().unwrap();

    let plan = plan_for_rename(&connection, root, &old_path, &new_path, vec![source_path], None)
        .unwrap();

    assert_eq!(plan[0].content, "# Title\n\nsee [[New]]\n");
    assert_eq!(plan[0].original, raw);
    assert_eq!(plan[0].original_hash, blake3::hash(raw.as_bytes()).to_hex().to_string());
}

#[test]
fn skips_unrelated_notes_without_changing_crlf_or_unicode_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let old_path = root.join("Old.md");
    let new_path = root.join("New.md");
    let unrelated_path = root.join("Unrelated.md");
    let linked_path = root.join("Linked.md");
    let unrelated = "Заметка 😀\r\nMarkdown [label](other.md)\r\n";
    fs::write(&old_path, "target").unwrap();
    fs::write(&unrelated_path, unrelated).unwrap();
    fs::write(&linked_path, "[[Old]] and `[[Old]]`").unwrap();

    let mut connection = Connection::open_in_memory().unwrap();
    open_schema(&connection).unwrap();
    let transaction = connection.transaction().unwrap();
    index_document(&transaction, root, &old_path, "target").unwrap();
    index_document(&transaction, root, &linked_path, "[[Old]] and `[[Old]]`").unwrap();
    transaction.commit().unwrap();

    let plan = plan_for_rename(
        &connection,
        root,
        &old_path,
        &new_path,
        vec![unrelated_path.clone(), linked_path.clone()],
        None,
    )
    .unwrap();

    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].path, linked_path);
    assert_eq!(plan[0].content, "[[New]] and `[[Old]]`");
    assert_eq!(fs::read_to_string(&unrelated_path).unwrap(), unrelated);
}

#[test]
fn fallback_candidates_keep_same_parent_preference_and_slash_style() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let near_dir = root.join("A");
    let far_dir = root.join("B");
    fs::create_dir_all(&near_dir).unwrap();
    fs::create_dir_all(&far_dir).unwrap();
    let old_path = near_dir.join("Same.md");
    let new_path = near_dir.join("Renamed.md");
    let source_path = near_dir.join("Source.md");
    fs::write(&source_path, "[[Same]] [[A/Same]] [[B/Same]]").unwrap();

    let candidates = vec![
        far_dir.join("Same.md").to_string_lossy().into_owned(),
        old_path.to_string_lossy().into_owned(),
    ];
    let connection = Connection::open_in_memory().unwrap();
    open_schema(&connection).unwrap();

    let plan = plan_for_rename(
        &connection,
        root,
        &old_path,
        &new_path,
        vec![source_path],
        Some(&candidates),
    )
    .unwrap();

    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].content, "[[Renamed]] [[A/Renamed]] [[B/Same]]");
}

#[test]
fn candidate_selection_keeps_parent_and_lexical_tiebreakers() {
    use super::target::{pick_candidate, select_candidate};
    use std::path::Path;
    let root = Path::new("vault");
    let source = root.join("notes/Source.md");
    let nearby = root.join("notes/Note.md");
    let remote = root.join("other/Note.md");
    let alternatives = vec![
        remote.to_string_lossy().into_owned(),
        nearby.to_string_lossy().into_owned(),
    ];
    assert_eq!(select_candidate(root, &source, &alternatives), Some(nearby));
    let ties = ["vault/notes/Beta.md", "vault/notes/Alpha.md"];
    assert_eq!(pick_candidate(root, &source, &ties), Some("vault/notes/Alpha.md"));
}

#[test]
fn sparse_unicode_openers_keep_original_offsets() {
    let prefix = "[regular markdown] 🦈 ".repeat(2_000);
    let source = format!("{prefix}`[[ignored]]` [[Цель#Заголовок|подпись]]");
    let links = super::parser::extract(&source);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target, "Цель#Заголовок");
    assert_eq!(links[0].offset_utf16, format!("{prefix}`[[ignored]]` ").encode_utf16().count());
    assert_eq!(&source[links[0].note_range.clone()], "Цель");
}

#[test]
fn escaped_overlapping_openers_preserve_existing_target_selection() {
    let links = super::parser::extract(r"\[[[Note]] [[Accepted]]");
    let targets = links.iter().map(|link| link.target.as_str()).collect::<Vec<_>>();
    assert_eq!(targets, ["Note", "Accepted"]);
}
